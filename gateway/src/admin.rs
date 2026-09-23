//! Admin endpoints: /healthz, /readyz, /metrics.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use color_eyre::eyre;
use common::http::{self, Response};
use tokio::net::TcpListener;

use crate::metrics::Metrics;

/// `min_connected` is the tunnel-member floor for readiness: 1 when a
/// tunnel is configured (the gateway cannot serve without a member), 0 in
/// direct mode.
pub async fn serve(
    listener: TcpListener,
    metrics: Arc<Metrics>,
    ready: Arc<AtomicBool>,
    min_connected: u64,
) -> eyre::Result<()> {
    http::serve_admin(listener, move |path| match path {
        "/healthz" => Response::text("200 OK", "ok\n"),
        "/readyz" => {
            if ready.load(Ordering::Relaxed)
                && metrics.tunnel_connected.load(Ordering::Relaxed) >= min_connected
            {
                Response::text("200 OK", "ready\n")
            } else {
                Response::text("503 Service Unavailable", "not ready\n")
            }
        }
        "/metrics" => Response::prometheus(metrics.render()),
        _ => Response::not_found(),
    })
    .await
}
