use std::io::IsTerminal;

use tracing_error::ErrorLayer;
use tracing_subscriber::{
    Layer as _,
    filter::{LevelFilter, Targets},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
};

/// Install the process-wide subscriber: human-readable on a terminal,
/// flattened JSON otherwise (for machine consumption, e.g. customer SIEMs).
pub fn init() {
    let env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    // The payload-dumping websocket targets are masked by a second filter
    // layer ANDed with the env filter, so no RUST_LOG directive — however
    // specific — can re-enable them.
    let payload_off = Targets::new()
        .with_default(LevelFilter::TRACE)
        .with_targets(
            tunnel::ws::PAYLOAD_LOGGING_TARGETS
                .iter()
                .map(|target| (*target, LevelFilter::OFF)),
        );

    let registry = tracing_subscriber::registry().with(ErrorLayer::default());
    if std::io::stdout().is_terminal() {
        registry
            .with(
                tracing_subscriber::fmt::layer()
                    .with_filter(env_filter)
                    .with_filter(payload_off),
            )
            .init();
    } else {
        registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .with_filter(env_filter)
                    .with_filter(payload_off),
            )
            .init();
    }
}
