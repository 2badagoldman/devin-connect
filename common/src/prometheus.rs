//! Prometheus text exposition format.

use std::sync::atomic::{AtomicU64, Ordering};

pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4";

pub fn counter(name: &str, help: &str, value: &AtomicU64) -> String {
    sample("counter", name, help, value)
}

pub fn gauge(name: &str, help: &str, value: &AtomicU64) -> String {
    sample("gauge", name, help, value)
}

fn sample(kind: &str, name: &str, help: &str, value: &AtomicU64) -> String {
    let value = value.load(Ordering::Relaxed);
    format!("# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}\n")
}
