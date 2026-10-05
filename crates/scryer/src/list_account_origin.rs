use std::net::SocketAddr;

use scryer_interface::context::RequestListAccountLinkOrigins;
use url::Url;

/// Callback destinations approved by server configuration. Request headers
/// cannot enlarge this set, including when the request arrives through a proxy.
#[derive(Clone, Default)]
pub(crate) struct ListAccountOriginPolicy {
    origins: RequestListAccountLinkOrigins,
}

impl ListAccountOriginPolicy {
    pub(crate) fn from_env(bind: &str) -> Self {
        let public_url = std::env::var("SCRYER_PUBLIC_URL").ok();
        let tls_enabled = ["SCRYER_TLS_CERT", "SCRYER_TLS_KEY"]
            .iter()
            .all(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()));
        let Ok(bind) = bind.parse() else {
            return Self::default();
        };
        Self::from_config(public_url.as_deref(), bind, tls_enabled)
    }

    pub(crate) fn from_config(
        public_url: Option<&str>,
        bind: SocketAddr,
        tls_enabled: bool,
    ) -> Self {
        let mut origins = Vec::new();
        if let Some(value) = public_url.map(str::trim).filter(|value| !value.is_empty()) {
            let Some(origin) = configured_origin(value) else {
                tracing::warn!("account linking disabled because SCRYER_PUBLIC_URL is invalid");
                return Self::default();
            };
            origins.push(origin);
        }

        // A wildcard listener does not approve every address that can reach it.
        // DNS and reverse-proxy addresses require the configured public URL.
        if bind.port() != 0 {
            let scheme = if tls_enabled { "https" } else { "http" };
            if !bind.ip().is_unspecified() {
                push_origin(&mut origins, &format!("{scheme}://{bind}"));
            }
            for host in ["localhost", "127.0.0.1", "[::1]"] {
                push_origin(&mut origins, &format!("{scheme}://{host}:{}", bind.port()));
            }
        }

        Self {
            origins: RequestListAccountLinkOrigins(origins.into()),
        }
    }

    pub(crate) fn request_context(&self) -> RequestListAccountLinkOrigins {
        self.origins.clone()
    }
}

fn configured_origin(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_some_and(|host| host.contains('*'))
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn push_origin(origins: &mut Vec<String>, value: &str) {
    if let Some(origin) = configured_origin(value)
        && !origins.contains(&origin)
    {
        origins.push(origin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origins(public_url: Option<&str>, bind: &str, tls_enabled: bool) -> Vec<String> {
        ListAccountOriginPolicy::from_config(
            public_url,
            bind.parse().expect("bind address"),
            tls_enabled,
        )
        .request_context()
        .0
        .to_vec()
    }

    #[test]
    fn account_link_origin_approves_configured_private_and_proxy_urls_with_base_paths() {
        for (url, expected) in [
            ("https://media.home/scryer/", "https://media.home"),
            ("http://192.168.1.20:8080/base", "http://192.168.1.20:8080"),
            ("https://[fd00::20]:8443/scryer", "https://[fd00::20]:8443"),
            ("https://MEDIA.HOME:443/scryer", "https://media.home"),
        ] {
            let approved = origins(Some(url), "127.0.0.1:8080", false);
            assert!(approved.iter().any(|origin| origin == expected));
            assert!(
                !approved
                    .iter()
                    .any(|origin| origin == "https://attacker.invalid")
            );
        }
    }

    #[test]
    fn account_link_origin_approves_only_literal_bind_and_loopback_defaults() {
        let approved = origins(None, "192.168.1.20:8443", true);
        assert!(approved.contains(&"https://192.168.1.20:8443".into()));
        assert!(approved.contains(&"https://localhost:8443".into()));
        assert!(approved.contains(&"https://127.0.0.1:8443".into()));
        assert!(approved.contains(&"https://[::1]:8443".into()));
        assert!(!approved.contains(&"http://192.168.1.20:8443".into()));
        assert!(!approved.contains(&"https://192.168.1.21:8443".into()));
        assert!(!approved.contains(&"https://media.home:8443".into()));
        assert!(origins(None, "127.0.0.1:80", false).contains(&"http://localhost".into()));
    }

    #[test]
    fn account_link_origin_wildcard_bind_does_not_approve_lan_or_dns_addresses() {
        for bind in ["0.0.0.0:8080", "[::]:8080"] {
            let approved = origins(None, bind, false);
            assert_eq!(approved.len(), 3);
            assert!(!approved.contains(&"http://192.168.1.20:8080".into()));
            assert!(!approved.contains(&"http://media.home:8080".into()));
        }
        assert!(origins(None, "127.0.0.1:0", false).is_empty());
    }

    #[test]
    fn account_link_origin_invalid_public_url_fails_closed_even_for_local_defaults() {
        for value in [
            "not a URL",
            "ftp://media.home",
            "https://user:secret@media.home",
            "https://media.home?callback=attacker",
            "https://media.home#callback",
            "https://*.home",
            "*",
        ] {
            assert!(origins(Some(value), "127.0.0.1:8080", false).is_empty());
        }
    }
}
