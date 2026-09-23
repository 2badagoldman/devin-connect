use std::{net::SocketAddr, time::Duration};

use tokio::net::{TcpListener, TcpStream};
use tracing::warn;

const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(100);

/// Accept the next connection. Transient accept errors (EMFILE,
/// ECONNABORTED, ...) are logged and retried after a short backoff so one
/// failure never takes the listener down.
pub async fn accept(listener: &TcpListener, what: &str) -> (TcpStream, SocketAddr) {
    loop {
        match listener.accept().await {
            Ok(accepted) => return accepted,
            Err(err) => {
                warn!(error = %err, "{what} accept failed");
                tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
            }
        }
    }
}
