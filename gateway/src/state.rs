//! Compiled runtime state and atomic reload.
//!
//! A config document is compiled into an immutable [`EngineState`]; the
//! serving loop swaps the active state atomically via a watch channel.
//! In-flight connections keep the state they were accepted with. A reload
//! that fails to parse, validate, or compile is rejected as a whole and the
//! last-known-good state stays active.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use color_eyre::eyre::{self, WrapErr};
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::{config::GatewayConfig, dns::HostResolver, metrics::Metrics, policy::PolicyEngine};

/// Immutable compiled state derived from one config document.
pub struct EngineState {
    pub config: GatewayConfig,
    pub policy: PolicyEngine,
    pub resolver: HostResolver,
}

impl EngineState {
    pub fn compile(config: GatewayConfig) -> eyre::Result<Self> {
        let policy = PolicyEngine::new(&config.routes)?;
        let resolver = HostResolver::new(&config.dns)?;
        Ok(Self {
            config,
            policy,
            resolver,
        })
    }
}

/// Loads, compiles, and atomically publishes config states.
pub struct StateHandle {
    config_path: PathBuf,
    last_good_yaml: String,
    tx: watch::Sender<Arc<EngineState>>,
    metrics: Arc<Metrics>,
}

impl StateHandle {
    /// Load the initial config. Failure here means the gateway must not
    /// start (fail closed).
    pub async fn load_initial(
        config_path: &Path,
        metrics: Arc<Metrics>,
    ) -> eyre::Result<(Self, watch::Receiver<Arc<EngineState>>)> {
        let (config, yaml) = GatewayConfig::load(config_path).await?;
        let state = EngineState::compile(config).wrap_err("failed to compile config")?;
        set_config_version(&metrics.config_version, &state);
        let (tx, rx) = watch::channel(Arc::new(state));
        let handle = Self {
            config_path: config_path.to_path_buf(),
            last_good_yaml: yaml,
            tx,
            metrics,
        };
        Ok((handle, rx))
    }

    /// Attempt a reload from the config path. On any failure the active
    /// state is kept and the error is returned (and counted).
    pub async fn reload(&mut self) -> eyre::Result<bool> {
        let result = self.try_reload().await;
        match &result {
            Ok(true) => {
                self.metrics
                    .config_reloads_total
                    .fetch_add(1, Ordering::Relaxed);
                info!(config_path = %self.config_path.display(), "config reloaded");
            }
            Ok(false) => {}
            Err(err) => {
                self.metrics
                    .config_reload_errors_total
                    .fetch_add(1, Ordering::Relaxed);
                error!(
                    config_path = %self.config_path.display(),
                    error = format!("{err:#}"),
                    "config reload rejected; keeping last-known-good config"
                );
            }
        }
        result
    }

    async fn try_reload(&mut self) -> eyre::Result<bool> {
        let (config, yaml) = GatewayConfig::load(&self.config_path).await?;
        if self.last_good_yaml == yaml {
            return Ok(false);
        }
        let state = EngineState::compile(config).wrap_err("failed to compile config")?;
        {
            let active = self.tx.borrow();
            if state.config.listen != active.config.listen
                || state.config.admin_listen != active.config.admin_listen
            {
                warn!("listen addresses changed in config; a restart is required to apply them");
            }
            // Pool size is the one tunnel setting applied live (the pool
            // supervisor watches the state channel); everything else in the
            // stanza is baked into established connections.
            let restart_only_changed = match (&state.config.tunnel, &active.config.tunnel) {
                (Some(new), Some(old)) => {
                    let mut resized_old = old.clone();
                    resized_old.connection_pool.size = new.connection_pool.size;
                    *new != resized_old
                }
                (None, None) => false,
                _ => true,
            };
            if restart_only_changed {
                warn!(
                    "tunnel settings other than connection_pool.size changed in config; \
                     a restart is required to apply them"
                );
            }
        }
        set_config_version(&self.metrics.config_version, &state);
        self.tx.send_replace(Arc::new(state));
        self.last_good_yaml = yaml;
        Ok(true)
    }
}

fn set_config_version(gauge: &AtomicU64, state: &EngineState) {
    gauge.store(state.config.config_version.unwrap_or(0), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_yaml(version: u64, hostname: &str) -> String {
        format!(
            r#"
schema_version: 1
config_version: {version}
listen: 127.0.0.1:18443
admin_listen: 127.0.0.1:19090
routes:
  - name: r
    rules: [{{hostname: "{hostname}"}}]
"#
        )
    }

    #[tokio::test]
    async fn reload_swaps_state_and_keeps_last_good_on_failure() {
        let dir = std::env::temp_dir().join(format!("gateway-state-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gateway.yaml");
        std::fs::write(&path, config_yaml(1, "one.corp.example")).unwrap();

        let metrics = Arc::new(Metrics::default());
        let (mut handle, rx) = StateHandle::load_initial(&path, metrics.clone())
            .await
            .unwrap();
        assert_eq!(rx.borrow().config.config_version, Some(1));
        assert_eq!(metrics.config_version.load(Ordering::Relaxed), 1);

        // Unchanged file: no swap.
        assert!(!handle.reload().await.unwrap());

        // Valid new config: swapped atomically.
        std::fs::write(&path, config_yaml(2, "two.corp.example")).unwrap();
        assert!(handle.reload().await.unwrap());
        assert_eq!(rx.borrow().config.config_version, Some(2));
        assert!(
            rx.borrow()
                .policy
                .find_route(&crate::policy::DestHost::Name("two.corp.example"), 443)
                .is_some()
        );
        assert_eq!(metrics.config_reloads_total.load(Ordering::Relaxed), 1);

        // Invalid config: rejected, last-known-good stays active.
        std::fs::write(&path, "schema_version: 99\n").unwrap();
        assert!(handle.reload().await.is_err());
        assert_eq!(rx.borrow().config.config_version, Some(2));
        assert!(
            rx.borrow()
                .policy
                .find_route(&crate::policy::DestHost::Name("two.corp.example"), 443)
                .is_some()
        );
        assert_eq!(
            metrics.config_reload_errors_total.load(Ordering::Relaxed),
            1
        );
        assert_eq!(metrics.config_version.load(Ordering::Relaxed), 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn initial_load_fails_closed_on_invalid_config() {
        let dir = std::env::temp_dir().join(format!("gateway-state-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gateway.yaml");
        std::fs::write(&path, "not: valid\n").unwrap();
        let metrics = Arc::new(Metrics::default());
        assert!(StateHandle::load_initial(&path, metrics).await.is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
