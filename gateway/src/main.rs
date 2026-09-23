#![forbid(unsafe_code)]

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use argh::FromArgs;
use color_eyre::eyre::{self, WrapErr, bail};
use common::config_dir;
use gateway::{
    admin,
    config::GatewayConfig,
    datapath,
    metrics::Metrics,
    state::{EngineState, StateHandle},
    tunnel_client::TunnelClient,
};
use tokio::net::TcpListener;
use tracing::info;

const RELOAD_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Devin customer gateway datapath.
#[derive(Debug, FromArgs)]
struct Args {
    #[argh(subcommand)]
    command: Command,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
enum Command {
    Serve(ServeCmd),
    Doctor(DoctorCmd),
    ValidateConfig(ValidateConfigCmd),
}

/// Run the gateway.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "serve")]
struct ServeCmd {
    /// config file to load
    #[argh(option)]
    config: PathBuf,
}

/// Validate the config and report a diagnostic summary as JSON.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "doctor")]
struct DoctorCmd {
    /// config file to load
    #[argh(option)]
    config: PathBuf,

    /// hostname to resolve with the configured DNS settings (repeatable)
    #[argh(option)]
    resolve: Vec<String>,
}

/// Validate the config and exit.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "validate-config")]
struct ValidateConfigCmd {
    /// config file to load
    #[argh(option)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    common::logging::init();
    color_eyre::install()?;

    let args: Args = argh::from_env();
    match args.command {
        Command::Serve(cmd) => serve(cmd).await,
        Command::Doctor(cmd) => doctor(cmd).await,
        Command::ValidateConfig(cmd) => validate_config(cmd).await,
    }
}

async fn serve(cmd: ServeCmd) -> eyre::Result<()> {
    let metrics = Arc::new(Metrics::default());

    // Fail closed: an invalid config means the gateway does not start.
    let (handle, state_rx) = StateHandle::load_initial(&cmd.config, metrics.clone())
        .await
        .wrap_err("failed to load initial config")?;

    let (listen, admin_listen, tunnel) = {
        let state = state_rx.borrow();
        (
            state.config.listen,
            state.config.admin_listen,
            state.config.tunnel.clone(),
        )
    };
    // Direct mode only: in tunnel mode CONNECT requests arrive solely over
    // the tunnel (config validation rejects `listen` there).
    let listener = match listen {
        Some(listen) => Some(
            TcpListener::bind(listen)
                .await
                .wrap_err_with(|| format!("failed to bind data listener on {listen}"))?,
        ),
        None => None,
    };
    let admin_listener = TcpListener::bind(admin_listen)
        .await
        .wrap_err_with(|| format!("failed to bind admin listener on {admin_listen}"))?;
    match listen {
        Some(listen) => info!(%listen, %admin_listen, "gateway listening (direct mode)"),
        None => info!(%admin_listen, "gateway listening (tunnel mode)"),
    }

    let ready = Arc::new(AtomicBool::new(false));
    // With a tunnel configured the gateway is only useful once at least one
    // pool member is connected; readiness reflects that.
    let min_connected = u64::from(tunnel.is_some());
    let admin_task = tokio::spawn(admin::serve(
        admin_listener,
        metrics.clone(),
        ready.clone(),
        min_connected,
    ));
    let reload_task = tokio::spawn(reload_loop(handle));
    let tunnel_task = match tunnel {
        Some(tunnel) => {
            let client = TunnelClient::build(tunnel, config_dir(&cmd.config))
                .await
                .wrap_err("failed to set up tunnel client")?;
            tokio::spawn(client.run(state_rx.clone(), metrics.clone()))
        }
        // Direct mode: park the select arm on a task that never finishes.
        None => tokio::spawn(std::future::pending()),
    };
    ready.store(true, Ordering::Relaxed);

    let datapath_task = match listener {
        Some(listener) => tokio::spawn(datapath::serve(listener, state_rx, metrics)),
        None => tokio::spawn(std::future::pending()),
    };

    // None of these return in normal operation; if one stops (error or
    // panic), exit loudly rather than limp along without it.
    tokio::select! {
        result = datapath_task => bail!("datapath exited unexpectedly: {result:?}"),
        result = admin_task => bail!("admin server exited unexpectedly: {result:?}"),
        result = reload_task => bail!("config reload loop exited unexpectedly: {result:?}"),
        result = tunnel_task => bail!("tunnel client exited unexpectedly: {result:?}"),
    }
}

/// Reload the config when it changes on disk (checked periodically and on
/// SIGHUP). Failed reloads keep the last-known-good config.
async fn reload_loop(mut handle: StateHandle) {
    #[cfg(unix)]
    let mut hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        .expect("failed to install SIGHUP handler");
    loop {
        #[cfg(unix)]
        tokio::select! {
            _ = hangup.recv() => {}
            _ = tokio::time::sleep(RELOAD_POLL_INTERVAL) => {}
        }
        #[cfg(not(unix))]
        tokio::time::sleep(RELOAD_POLL_INTERVAL).await;

        // Errors are logged and counted inside; the active config is kept.
        let _ = handle.reload().await;
    }
}

async fn doctor(cmd: DoctorCmd) -> eyre::Result<()> {
    let mut ok = true;
    let (config, _yaml) = GatewayConfig::load(&cmd.config).await?;
    let state = EngineState::compile(config).wrap_err("failed to compile config")?;

    let routes: Vec<serde_json::Value> = state
        .config
        .routes
        .iter()
        .map(|route| {
            serde_json::json!({
                "name": route.name,
                "rules": route.rules.len(),
                "ports": route.ports,
                "upstream_proxy": route.upstream_proxy.as_ref().map(|p| p.addr.clone()),
            })
        })
        .collect();

    let mut resolutions = Vec::new();
    for host in &cmd.resolve {
        match state.resolver.resolve(host).await {
            Ok(addrs) => resolutions.push(serde_json::json!({
                "host": host,
                "addrs": addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            })),
            Err(err) => {
                ok = false;
                resolutions.push(serde_json::json!({
                    "host": host,
                    "error": format!("{err:#}"),
                }));
            }
        }
    }

    let report = serde_json::json!({
        "ok": ok,
        "config": {
            "path": cmd.config.display().to_string(),
            "schema_version": state.config.schema_version,
            "config_version": state.config.config_version,
            "listen": state.config.listen.map(|listen| listen.to_string()),
            "admin_listen": state.config.admin_listen.to_string(),
            "dns": state.resolver.describe(),
            "tunnel": state.config.tunnel.as_ref().map(|tunnel| serde_json::json!({
                "endpoint": tunnel.endpoint,
                "gateway_id": tunnel.gateway_id,
            })),
        },
        "routes": routes,
        "resolutions": resolutions,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);

    eyre::ensure!(ok, "doctor found problems");
    Ok(())
}

async fn validate_config(cmd: ValidateConfigCmd) -> eyre::Result<()> {
    let (config, _yaml) = GatewayConfig::load(&cmd.config).await?;
    let tunnel = config.tunnel.clone();
    EngineState::compile(config).wrap_err("failed to compile config")?;
    // Check the tunnel's file references (token, CA bundle) the same way
    // serve would.
    if let Some(tunnel) = tunnel {
        TunnelClient::build(tunnel, config_dir(&cmd.config))
            .await
            .wrap_err("invalid tunnel config")?;
    }
    println!("config ok");
    Ok(())
}
