//! Wire protocol shared by the customer gateway (tunnel client) and the
//! Devin-side edge (tunnel terminator). One implementation, used by both
//! ends, tested against itself.
//!
//! Layering, bottom-up:
//!
//! ```text
//! TCP (optionally through a customer egress proxy)
//!   -> TLS
//!   -> hello exchange (one JSON line each way; authenticates the gateway)
//!   -> yamux stream multiplexing
//! ```
//!
//! Stream conventions on an established tunnel:
//! - The gateway opens exactly one stream: the control stream, carrying
//!   JSON-line heartbeats (gateway -> edge) and acks (edge -> gateway).
//! - The edge opens one stream per proxied connection and relays the
//!   caller's HTTP CONNECT request verbatim; the gateway serves it with
//!   the same datapath as a locally accepted connection, so policy and
//!   authorization stay gateway-side. Stream payload is opaque to the
//!   tunnel: customer bytes are spliced, never parsed.

#![forbid(unsafe_code)]

pub mod mux;
pub mod protocol;
pub mod ws;
