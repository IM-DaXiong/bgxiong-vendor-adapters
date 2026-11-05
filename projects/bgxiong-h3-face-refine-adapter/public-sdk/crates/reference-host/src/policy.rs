//! One platform-neutral outbound gate for buffered and streaming HTTP.
use crate::media::LeaseScope;
use bgx_adapter_v2_ports::wire::host::HostError;
use bgx_adapter_v2_validator::package::VerifiedPackage;
use ipnet::IpNet;
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use tokio_util::sync::CancellationToken;
use url::Url;
pub type Result<T> = std::result::Result<T, HostError>;
pub(crate) fn error(code: &str, message: &str) -> HostError {
    serde_json::from_value(json!({"code":code,"message":message})).expect("public host error")
}
pub(crate) fn protocol() -> &'static Value {
    static VALUE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    VALUE.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../protocol/vendor-adapter-protocol.json"
        ))
        .expect("public protocol")
    })
}
#[derive(Clone)]
pub struct DevelopmentNetworkGrant {
    pub origins: Vec<String>,
    pub allow_private: bool,
    pub allow_embedded_authorization: bool,
    pub denied_hosts: Vec<String>,
    pub denied_networks: Vec<IpNet>,
}
#[derive(Clone)]
pub struct NetworkPolicy {
    scope: LeaseScope,
    origins: Vec<String>,
    allow_private: bool,
    embedded_authorization: bool,
    active: CancellationToken,
    enabled: bool,
    denied_hosts: Vec<String>,
    denied_networks: Vec<IpNet>,
    auth_modes: Vec<String>,
}
pub struct Destination {
    pub url: Url,
    pub addresses: Vec<SocketAddr>,
    pub origin: String,
}
pub fn host_matches(host: &str, pattern: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern
    }
}
pub fn protected_header(name: &str) -> bool {
    protocol()["security"]["protectedHeaders"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(name)))
}
pub fn parse_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| error("PERMISSION_DENIED", "Outbound URL is invalid"))?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(error(
            "PERMISSION_DENIED",
            "Outbound URL scheme, authority or fragment is forbidden",
        ));
    }
    Ok(url)
}
impl NetworkPolicy {
    pub fn new(
        package: &VerifiedPackage,
        scope: LeaseScope,
        grant: DevelopmentNetworkGrant,
        network_enabled: bool,
    ) -> Result<Self> {
        if package.identity() != &scope.package
            || scope.binding_id.is_empty()
            || scope.request_id.is_empty()
        {
            return Err(error(
                "PERMISSION_DENIED",
                "Network scope does not match the verified package",
            ));
        }
        let caps = &package.manifest()["requestedCapabilities"];
        let policy = caps["endpointPolicy"].as_array().unwrap();
        let configurable = policy.iter().any(|v| v == "userConfigurable");
        let static_hosts = policy.iter().any(|v| v == "staticHosts");
        let mut origins = vec![];
        for origin in grant.origins {
            let url = parse_url(&origin)?;
            if url.path() != "/" || url.query().is_some() {
                return Err(error(
                    "INVALID_INPUT",
                    "Development network grants must name an exact origin",
                ));
            }
            if !configurable
                && !(static_hosts
                    && caps["networkHosts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| host_matches(url.host_str().unwrap(), v.as_str().unwrap())))
            {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Development destination was not requested by the manifest",
                ));
            }
            origins.push(url.origin().ascii_serialization());
        }
        let allow_private = grant.allow_private
            && caps["allowPrivateNetwork"] == true
            && policy.iter().any(|v| v == "allowPrivateNetwork");
        let embedded_authorization = grant.allow_embedded_authorization
            && !package.is_trusted()
            && package.identity().plugin_id.starts_with("local.")
            && caps["embeddedHeaders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "authorization");
        Ok(Self {
            scope,
            origins,
            allow_private,
            embedded_authorization,
            active: CancellationToken::new(),
            enabled: network_enabled,
            denied_hosts: grant.denied_hosts,
            denied_networks: grant.denied_networks,
            auth_modes: caps["auth"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().into())
                .collect(),
        })
    }
    pub fn scope(&self) -> &LeaseScope {
        &self.scope
    }
    pub fn auth_modes(&self) -> &[String] {
        &self.auth_modes
    }
    pub fn origins(&self) -> &[String] {
        &self.origins
    }
    pub fn permits_embedded_authorization(&self) -> bool {
        self.embedded_authorization
    }
    pub fn revoke(&self) {
        self.active.cancel();
    }
    pub async fn revoked(&self) {
        self.active.cancelled().await
    }
    pub fn check(&self, scope: &LeaseScope) -> Result<()> {
        if scope != &self.scope || self.active.is_cancelled() || !self.enabled {
            return Err(error(
                "PERMISSION_DENIED",
                "Network permission is unavailable for this live request",
            ));
        }
        Ok(())
    }
    pub fn check_url(
        &self,
        scope: &LeaseScope,
        raw: &str,
        addresses: &[IpAddr],
    ) -> Result<Destination> {
        self.check(scope)?;
        let url = parse_url(raw)?;
        let origin = url.origin().ascii_serialization();
        let host = url.host_str().unwrap();
        if !self.origins.contains(&origin)
            || self.denied_hosts.iter().any(|p| host_matches(host, p))
            || addresses.is_empty()
        {
            return Err(error(
                "PERMISSION_DENIED",
                "Outbound origin, host or DNS result is not authorized",
            ));
        }
        let normalize = |ip: IpAddr| match ip {
            IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v)),
            ip => ip,
        };
        let mut all_private = true;
        for ip in addresses.iter().copied().map(normalize) {
            let within = |name: &str| {
                protocol()["security"][name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| {
                        v.as_str()
                            .unwrap()
                            .parse::<IpNet>()
                            .expect("public CIDR")
                            .contains(&ip)
                    })
            };
            if within("alwaysDeniedNetworks")
                || self.denied_networks.iter().any(|n| n.contains(&ip))
            {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Outbound address belongs to a forbidden network",
                ));
            }
            let private = within("privateNetworks");
            all_private &= private;
            if private && !self.allow_private {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Private network access was not explicitly granted",
                ));
            }
        }
        if url.scheme() == "http" && (!self.allow_private || !all_private) {
            return Err(error(
                "PERMISSION_DENIED",
                "Plain HTTP is only permitted on explicitly granted private networks",
            ));
        }
        let port = url
            .port_or_known_default()
            .ok_or_else(|| error("PERMISSION_DENIED", "Outbound port is unavailable"))?;
        Ok(Destination {
            url,
            origin,
            addresses: addresses
                .iter()
                .map(|ip| SocketAddr::new(*ip, port))
                .collect(),
        })
    }
    pub async fn resolve(&self, scope: &LeaseScope, raw: &str) -> Result<Destination> {
        self.check(scope)?;
        let url = parse_url(raw)?;
        // Reject the origin before DNS, then screen EVERY answer and pin exactly
        // those addresses in reqwest. DNS is never repeated by the HTTP client.
        if !self.origins.contains(&url.origin().ascii_serialization()) {
            return Err(error(
                "PERMISSION_DENIED",
                "Outbound origin has no development grant",
            ));
        }
        let port = url
            .port_or_known_default()
            .ok_or_else(|| error("PERMISSION_DENIED", "Outbound port is invalid"))?;
        let ips = match url.host().unwrap() {
            url::Host::Ipv4(ip) => vec![IpAddr::V4(ip)],
            url::Host::Ipv6(ip) => vec![IpAddr::V6(ip)],
            url::Host::Domain(host) => tokio::net::lookup_host((host, port))
                .await
                .map_err(|_| error("RESOURCE_UNAVAILABLE", "Outbound DNS resolution failed"))?
                .map(|a| a.ip())
                .collect(),
        };
        self.check_url(scope, raw, &ips)
    }
}
/// Audit metadata deliberately excludes URL paths/query, header values, bodies,
/// credential IDs, signatures and upstream error strings.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpAudit {
    pub method: String,
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub status: Option<u16>,
    pub outcome: String,
}
impl HttpAudit {
    pub fn new(method: &str, url: &Url) -> Self {
        Self {
            method: method.into(),
            scheme: url.scheme().into(),
            host: url.host_str().unwrap_or("").into(),
            port: url.port_or_known_default().unwrap_or(0),
            status: None,
            outcome: "started".into(),
        }
    }
}
