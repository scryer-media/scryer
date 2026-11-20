//! Opt-in limits on the trust Scryer gives the network path when it serves a
//! request without a login: in authless mode, and through the local-IP login
//! skip. Both are off by default, and then every request is decided as before.
//!
//! - An allowed-host list. When it holds names, such a request must carry a
//!   `Host` naming an IP literal, `localhost`, the public URL's host or one of
//!   the names. A browser tricked by DNS rebinding sends the attacker's name,
//!   so it no longer receives local trust.
//! - Restricting local trust to listed proxies. When on, forwarding headers
//!   count only from a peer in the trusted proxy list, and a listed peer must
//!   forward a fully local client chain.

use std::net::IpAddr;
use std::sync::{Arc, RwLock};

pub const ALLOWED_HOSTS_KEY: &str = "auth.local_trust.allowed_hosts";
pub const RESTRICT_TO_TRUSTED_PROXIES_KEY: &str = "auth.local_trust.restrict_to_trusted_proxies";

/// The most names the allowed-host list holds.
pub const MAX_ALLOWED_HOSTS: usize = 64;

const MAX_HOST_NAME_LENGTH: usize = 253;
const MAX_HOST_LABEL_LENGTH: usize = 63;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalTrustPolicy {
    /// Normalized host names. Empty turns the host check off.
    pub allowed_hosts: Vec<String>,
    pub restrict_to_trusted_proxies: bool,
}

impl LocalTrustPolicy {
    /// Whether a request with this `Host` may receive local trust. With an
    /// empty list every request may, including one without a `Host`.
    pub fn allows_host(&self, request_host: Option<&str>, public_url_host: Option<&str>) -> bool {
        if self.allowed_hosts.is_empty() {
            return true;
        }
        let Some(host) = request_host.and_then(request_host_name) else {
            return false;
        };
        is_ip_literal(&host)
            || host == "localhost"
            || public_url_host
                .and_then(request_host_name)
                .is_some_and(|public_host| public_host == host)
            || self.allowed_hosts.iter().any(|allowed| *allowed == host)
    }
}

/// The host part of a `Host` header or URI authority: the port and IPv6
/// brackets removed, lowercased, without a trailing dot. `None` when the value
/// is not a host.
pub fn request_host_name(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.parse::<IpAddr>().is_ok() {
        return Some(raw.to_ascii_lowercase());
    }
    let host = if let Some(rest) = raw.strip_prefix('[') {
        let (inside, after) = rest.split_once(']')?;
        if !(after.is_empty() || port_suffix_is_valid(after)) {
            return None;
        }
        inside.parse::<std::net::Ipv6Addr>().ok()?;
        inside
    } else {
        match raw.rsplit_once(':') {
            Some((host, port)) if port_suffix_is_valid(&format!(":{port}")) => host,
            Some(_) => return None,
            None => raw,
        }
    };
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty() || host.contains(['/', '@', ' ', '[', ']']) {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

fn port_suffix_is_valid(suffix: &str) -> bool {
    suffix
        .strip_prefix(':')
        .is_some_and(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_ip_literal(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

/// Normalizes one name for the allowed-host list. Accepts a DNS name or an IP
/// literal without a scheme, port, path or wildcard.
pub fn normalize_allowed_host(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let host = trimmed
        .strip_suffix('.')
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    if host.is_empty() || host.len() > MAX_HOST_NAME_LENGTH {
        return None;
    }
    if is_ip_literal(&host) {
        return Some(host);
    }
    host.split('.')
        .all(|label| {
            !label.is_empty()
                && label.len() <= MAX_HOST_LABEL_LENGTH
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
        .then_some(host)
}

/// Normalizes a whole list, dropping blanks and repeats. `Err` carries the
/// first value that is not a host name, or reports a list over the limit.
pub fn normalize_allowed_hosts(values: &[String]) -> Result<Vec<String>, AllowedHostsError> {
    let mut hosts: Vec<String> = Vec::new();
    for value in values.iter().filter(|value| !value.trim().is_empty()) {
        let host = normalize_allowed_host(value)
            .ok_or_else(|| AllowedHostsError::Invalid(value.trim().to_string()))?;
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    if hosts.len() > MAX_ALLOWED_HOSTS {
        return Err(AllowedHostsError::TooMany);
    }
    Ok(hosts)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowedHostsError {
    Invalid(String),
    TooMany,
}

/// The local-trust policy that transport adapters read per request.
#[derive(Clone, Default)]
pub struct LocalTrustRuntime(Arc<RwLock<Arc<LocalTrustPolicy>>>);

impl LocalTrustRuntime {
    pub fn new(policy: LocalTrustPolicy) -> Self {
        Self(Arc::new(RwLock::new(Arc::new(policy))))
    }

    pub fn snapshot(&self) -> Arc<LocalTrustPolicy> {
        self.0
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn replace(&self, policy: LocalTrustPolicy) {
        *self.0.write().unwrap_or_else(|error| error.into_inner()) = Arc::new(policy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(hosts: &[&str]) -> LocalTrustPolicy {
        LocalTrustPolicy {
            allowed_hosts: hosts.iter().map(|host| host.to_string()).collect(),
            restrict_to_trusted_proxies: false,
        }
    }

    #[test]
    fn empty_list_allows_every_host() {
        let policy = LocalTrustPolicy::default();
        assert!(policy.allows_host(Some("attacker.example"), None));
        assert!(policy.allows_host(None, None));
    }

    #[test]
    fn listed_hosts_ip_literals_localhost_and_public_url_host_are_allowed() {
        let policy = policy(&["scryer.lan"]);
        for host in [
            "scryer.lan",
            "SCRYER.lan:8080",
            "scryer.lan.",
            "192.168.1.20",
            "192.168.1.20:8080",
            "[::1]:8080",
            "[fe80::1]",
            "::1",
            "localhost",
            "LOCALHOST:3000",
            "media.example.com:443",
        ] {
            assert!(
                policy.allows_host(Some(host), Some("media.example.com")),
                "{host} should be allowed"
            );
        }
    }

    #[test]
    fn rebinding_and_missing_hosts_are_refused_when_the_list_is_set() {
        let policy = policy(&["scryer.lan"]);
        for host in [
            "attacker.example",
            "attacker.example:8080",
            "scryer.lan.attacker.example",
            "sub.localhost",
            "",
            "bad host",
            "scryer.lan:port",
        ] {
            assert!(
                !policy.allows_host(Some(host), Some("media.example.com")),
                "{host} should be refused"
            );
        }
        assert!(!policy.allows_host(None, None));
        assert!(!policy.allows_host(Some("media.example.com"), None));
    }

    #[test]
    fn normalizes_and_validates_allowed_hosts() {
        assert_eq!(
            normalize_allowed_hosts(&[
                " Scryer.LAN. ".into(),
                "scryer.lan".into(),
                "".into(),
                "media_box".into(),
                "10.0.0.5".into(),
            ]),
            Ok(vec![
                "scryer.lan".into(),
                "media_box".into(),
                "10.0.0.5".into()
            ])
        );
        for value in [
            "http://scryer.lan",
            "scryer.lan:8080",
            "*.scryer.lan",
            "scryer..lan",
            "-scryer.lan",
            "scryer.lan/path",
            "[::1]",
        ] {
            assert_eq!(
                normalize_allowed_hosts(&[value.into()]),
                Err(AllowedHostsError::Invalid(value.into())),
                "{value}"
            );
        }
        let too_many: Vec<String> = (0..=MAX_ALLOWED_HOSTS)
            .map(|index| format!("host{index}.lan"))
            .collect();
        assert_eq!(
            normalize_allowed_hosts(&too_many),
            Err(AllowedHostsError::TooMany)
        );
    }
}
