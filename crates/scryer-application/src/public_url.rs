//! The instance's public URL, resolved once and read by every consumer.
//!
//! Resolution is environment variable, then saved setting, then none. List
//! account linking, OAuth metadata, and passkey defaults all read the same
//! runtime value, so a saved-setting change applies without a restart wherever
//! the consumer reads it per request.
use std::sync::{Arc, RwLock};

use url::{Host, Url};

pub const PUBLIC_URL_KEY: &str = "service.public_url";
pub const PUBLIC_URL_ENV: &str = "SCRYER_PUBLIC_URL";

/// Where an effective configuration value comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfigValueSource {
    Environment,
    Settings,
    #[default]
    Default,
}

impl ConfigValueSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Settings => "settings",
            Self::Default => "default",
        }
    }
}

/// Where the running passkey relying party comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PasskeyRelyingPartySource {
    /// `SCRYER_WEBAUTHN_RP_ID` and `SCRYER_WEBAUTHN_RP_ORIGIN`.
    Environment,
    /// Derived from the public URL because both variables were unset.
    PublicUrl,
    /// Passkeys are not configured.
    #[default]
    None,
}

impl PasskeyRelyingPartySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::PublicUrl => "public_url",
            Self::None => "none",
        }
    }
}

/// The configured public URL from both sources. The environment value wins
/// whenever it is set, even when it is invalid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublicUrlPolicy {
    environment: Option<String>,
    saved: Option<String>,
}

impl PublicUrlPolicy {
    pub fn new(environment: Option<&str>, saved: Option<&str>) -> Self {
        let clean = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Self {
            environment: clean(environment),
            saved: clean(saved),
        }
    }

    pub fn source(&self) -> ConfigValueSource {
        if self.environment.is_some() {
            ConfigValueSource::Environment
        } else if self.saved.is_some() {
            ConfigValueSource::Settings
        } else {
            ConfigValueSource::Default
        }
    }

    pub fn environment_value(&self) -> Option<&str> {
        self.environment.as_deref()
    }

    pub fn saved_value(&self) -> Option<&str> {
        self.saved.as_deref()
    }

    /// The value that wins resolution, valid or not.
    pub fn configured_value(&self) -> Option<&str> {
        self.environment.as_deref().or(self.saved.as_deref())
    }

    /// `Ok(None)` when nothing is configured; `Err` when the winning value is
    /// invalid.
    pub fn resolve(&self) -> Result<Option<Url>, String> {
        self.configured_value().map(parse_public_url).transpose()
    }

    pub fn url(&self) -> Option<Url> {
        self.resolve().ok().flatten()
    }

    pub fn origin(&self) -> Option<String> {
        self.url().map(|url| url.origin().ascii_serialization())
    }

    pub fn error(&self) -> Option<String> {
        self.resolve().err()
    }
}

/// Validate a public URL: absolute http or https, a host, no credentials, no
/// query or fragment, and no wildcard host. Any path is accepted here.
pub fn parse_public_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim())
        .map_err(|_| "the public URL must be an absolute http or https URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("the public URL must be an absolute http or https URL".into());
    }
    if url.host_str().is_some_and(|host| host.contains('*')) {
        return Err("the public URL must name a single host".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("the public URL must not include a username or password".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("the public URL must not include a query or fragment".into());
    }
    Ok(url)
}

/// Validate a value an administrator saves and return the stored form. The
/// path must be empty or equal to the base path the instance serves under.
pub fn normalize_saved_public_url(value: &str, base_path: &str) -> Result<String, String> {
    let url = parse_public_url(value)?;
    let path = url.path().trim_end_matches('/');
    let base_path = base_path.trim_end_matches('/');
    if !path.is_empty() && path != base_path {
        return Err(if base_path.is_empty() {
            "the public URL must not include a path because Scryer is served at the root".into()
        } else {
            format!("the public URL path must be empty or {base_path}")
        });
    }
    Ok(format!("{}{path}", url.origin().ascii_serialization()))
}

/// The passkey relying party a public URL implies, when browsers would accept
/// it: a domain name (not an IP address) over https, or `localhost`.
pub fn passkey_relying_party_from_public_url(url: &Url) -> Option<(String, Url)> {
    let Some(Host::Domain(domain)) = url.host() else {
        return None;
    };
    let domain = domain.to_ascii_lowercase();
    if url.scheme() != "https" && domain != "localhost" {
        return None;
    }
    let origin = Url::parse(&url.origin().ascii_serialization()).ok()?;
    Some((domain, origin))
}

/// Addressing facts fixed at process start, shown read-only to administrators.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstanceAddressing {
    /// Normalized base path, empty at the root.
    pub base_path: String,
    pub base_path_source: ConfigValueSource,
    pub bind_address: String,
    pub bind_source: ConfigValueSource,
    pub passkey_rp_id: Option<String>,
    pub passkey_rp_origin: Option<String>,
    pub passkey_rp_source: PasskeyRelyingPartySource,
}

/// Shared handle to the public URL policy and the startup addressing facts.
#[derive(Clone, Default)]
pub struct PublicUrlRuntime {
    policy: Arc<RwLock<Arc<PublicUrlPolicy>>>,
    addressing: Arc<RwLock<Arc<InstanceAddressing>>>,
}

impl PublicUrlRuntime {
    pub fn new(policy: PublicUrlPolicy, addressing: InstanceAddressing) -> Self {
        Self {
            policy: Arc::new(RwLock::new(Arc::new(policy))),
            addressing: Arc::new(RwLock::new(Arc::new(addressing))),
        }
    }

    pub fn snapshot(&self) -> Arc<PublicUrlPolicy> {
        self.policy
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn addressing(&self) -> Arc<InstanceAddressing> {
        self.addressing
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn replace(&self, policy: PublicUrlPolicy) {
        *self
            .policy
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Arc::new(policy);
    }

    pub(crate) fn install(&self, policy: PublicUrlPolicy, addressing: InstanceAddressing) {
        self.replace(policy);
        *self
            .addressing
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Arc::new(addressing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_wins_over_saved_setting_and_reports_its_source() {
        let both = PublicUrlPolicy::new(Some(" https://env.home "), Some("https://saved.home"));
        assert_eq!(both.source(), ConfigValueSource::Environment);
        assert_eq!(both.origin().as_deref(), Some("https://env.home"));
        assert_eq!(both.saved_value(), Some("https://saved.home"));

        let saved = PublicUrlPolicy::new(Some("   "), Some("https://saved.home"));
        assert_eq!(saved.source(), ConfigValueSource::Settings);
        assert_eq!(saved.origin().as_deref(), Some("https://saved.home"));

        let none = PublicUrlPolicy::new(None, None);
        assert_eq!(none.source(), ConfigValueSource::Default);
        assert_eq!(none.resolve(), Ok(None));
        assert!(none.error().is_none());
    }

    #[test]
    fn invalid_environment_value_still_wins_and_surfaces_an_error() {
        let policy = PublicUrlPolicy::new(Some("ftp://media.home"), Some("https://saved.home"));
        assert_eq!(policy.source(), ConfigValueSource::Environment);
        assert!(policy.url().is_none());
        assert!(policy.error().is_some());
    }

    #[test]
    fn rejects_credentials_queries_fragments_and_wildcards() {
        for value in [
            "not a URL",
            "ftp://media.home",
            "https://user:secret@media.home",
            "https://media.home?callback=attacker",
            "https://media.home#callback",
            "https://*.home",
            "/relative",
        ] {
            assert!(parse_public_url(value).is_err(), "{value}");
        }
        assert!(parse_public_url("http://192.168.1.20:8080/base").is_ok());
    }

    #[test]
    fn saved_path_must_match_the_base_path() {
        assert_eq!(
            normalize_saved_public_url("https://MEDIA.home:443/", "").as_deref(),
            Ok("https://media.home")
        );
        assert_eq!(
            normalize_saved_public_url("https://media.home/scryer/", "/scryer").as_deref(),
            Ok("https://media.home/scryer")
        );
        assert_eq!(
            normalize_saved_public_url("https://media.home", "/scryer").as_deref(),
            Ok("https://media.home")
        );
        assert!(normalize_saved_public_url("https://media.home/other", "/scryer").is_err());
        assert!(normalize_saved_public_url("https://media.home/scryer", "").is_err());
    }

    #[test]
    fn passkey_relying_party_requires_a_secure_domain() {
        let rp = |value: &str| {
            passkey_relying_party_from_public_url(&Url::parse(value).unwrap())
                .map(|(id, origin)| (id, origin.origin().ascii_serialization()))
        };
        assert_eq!(
            rp("https://Media.Home:8443/scryer"),
            Some(("media.home".into(), "https://media.home:8443".into()))
        );
        assert_eq!(
            rp("http://localhost:8080"),
            Some(("localhost".into(), "http://localhost:8080".into()))
        );
        assert_eq!(rp("http://media.home"), None);
        assert_eq!(rp("https://192.168.1.20"), None);
        assert_eq!(rp("https://[fd00::20]"), None);
    }

    #[test]
    fn requests_keep_one_snapshot_during_replacement() {
        let runtime = PublicUrlRuntime::default();
        runtime.replace(PublicUrlPolicy::new(None, Some("https://first.home")));
        let request = runtime.snapshot();
        runtime.replace(PublicUrlPolicy::new(None, Some("https://second.home")));
        assert_eq!(request.origin().as_deref(), Some("https://first.home"));
        assert_eq!(
            runtime.snapshot().origin().as_deref(),
            Some("https://second.home")
        );
    }
}
