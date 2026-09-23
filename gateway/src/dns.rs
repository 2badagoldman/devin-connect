//! Target hostname resolution.
//!
//! Targets are resolved at the gateway (inside the customer network), either
//! with explicitly configured nameservers or the system resolver.

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use color_eyre::eyre::{self, WrapErr, ensure};
use hickory_resolver::{
    Resolver, TokioResolver,
    config::{ConnectionConfig, NameServerConfig, ResolverConfig},
    net::runtime::TokioRuntimeProvider,
};

use crate::config::DnsConfig;

/// Resolves target hostnames to IP addresses.
pub enum HostResolver {
    /// The host OS resolver (getaddrinfo).
    System { timeout: Duration },
    /// Explicitly configured nameservers.
    Custom {
        resolver: Box<TokioResolver>,
        nameservers: Vec<SocketAddr>,
    },
}

impl HostResolver {
    pub fn new(config: &DnsConfig) -> eyre::Result<Self> {
        if config.nameservers.is_empty() {
            return Ok(Self::System {
                timeout: config.timeout,
            });
        }
        let nameservers = config
            .nameservers
            .iter()
            .map(|addr| {
                let mut udp = ConnectionConfig::udp();
                udp.port = addr.port();
                let mut tcp = ConnectionConfig::tcp();
                tcp.port = addr.port();
                NameServerConfig::new(addr.ip(), true, vec![udp, tcp])
            })
            .collect();
        let mut builder = Resolver::builder_with_config(
            ResolverConfig::from_parts(None, vec![], nameservers),
            TokioRuntimeProvider::default(),
        );
        builder.options_mut().timeout = config.timeout;
        Ok(Self::Custom {
            resolver: Box::new(builder.build().wrap_err("failed to build DNS resolver")?),
            nameservers: config.nameservers.clone(),
        })
    }

    pub async fn resolve(&self, host: &str) -> eyre::Result<Vec<IpAddr>> {
        let addrs: Vec<IpAddr> = match self {
            Self::System { timeout } => {
                tokio::time::timeout(*timeout, tokio::net::lookup_host((host, 0)))
                    .await
                    .wrap_err_with(|| format!("timed out resolving {host:?}"))?
                    .wrap_err_with(|| format!("failed to resolve {host:?}"))?
                    .map(|addr| addr.ip())
                    .collect()
            }
            Self::Custom { resolver, .. } => resolver
                .lookup_ip(host)
                .await
                .wrap_err_with(|| format!("failed to resolve {host:?}"))?
                .iter()
                .collect(),
        };
        ensure!(!addrs.is_empty(), "no addresses found for {host:?}");
        Ok(addrs)
    }

    /// Human-readable description for logs and doctor output.
    pub fn describe(&self) -> String {
        match self {
            Self::System { .. } => "system".to_string(),
            Self::Custom { nameservers, .. } => format!(
                "custom ({})",
                nameservers
                    .iter()
                    .map(|ns| ns.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn system_resolver_resolves_localhost() {
        let resolver = HostResolver::new(&DnsConfig::default()).unwrap();
        assert!(matches!(resolver, HostResolver::System { .. }));
        let addrs = resolver.resolve("localhost").await.unwrap();
        assert!(addrs.iter().any(|ip| ip.is_loopback()));
    }

    #[test]
    fn system_resolver_preserves_configured_timeout() {
        let config = DnsConfig {
            timeout: Duration::from_millis(123),
            ..DnsConfig::default()
        };
        let resolver = HostResolver::new(&config).unwrap();
        assert!(matches!(
            resolver,
            HostResolver::System { timeout } if timeout == config.timeout
        ));
    }

    #[tokio::test]
    async fn custom_resolver_uses_configured_nameservers() {
        let config = DnsConfig {
            nameservers: vec!["127.0.0.1:15353".parse().unwrap()],
            timeout: Duration::from_millis(200),
            allow_loopback: false,
        };
        let resolver = HostResolver::new(&config).unwrap();
        assert!(matches!(resolver, HostResolver::Custom { .. }));
        // Nothing is listening there: resolution must fail, not fall back to
        // the system resolver.
        assert!(resolver.resolve("db.corp.internal").await.is_err());
    }
}
