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
    pub fn resolve(&self) -> Result<Option<Url>, PublicUrlError> {
        self.configured_value().map(parse_public_url).transpose()
    }

    pub fn url(&self) -> Option<Url> {
        self.resolve().ok().flatten()
    }

    pub fn origin(&self) -> Option<String> {
        self.url().map(|url| url.origin().ascii_serialization())
    }

    pub fn error(&self) -> Option<PublicUrlError> {
        self.resolve().err()
    }
}

/// Stable reasons a public URL is rejected. Clients localize these; the
/// message is English for logs and API callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicUrlErrorCode {
    /// Not an absolute http or https URL with a host.
    InvalidUrl,
    /// The host contains a wildcard.
    WildcardHost,
    /// The URL carries a username or password.
    Credentials,
    /// The URL carries a query or fragment.
    QueryOrFragment,
    /// A path was given while the instance is served at the root.
    PathNotAllowed,
    /// The path differs from the base path the instance serves under.
    PathMismatch,
    /// The environment variable sets the public URL, so it cannot be changed here.
    EnvironmentLocked,
    /// A save and a reset were requested together.
    SaveAndReset,
    /// The change breaks registered passkeys and was not acknowledged.
    PasskeyAcknowledgementRequired,
}

impl PublicUrlErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUrl => "invalid_url",
            Self::WildcardHost => "wildcard_host",
            Self::Credentials => "credentials",
            Self::QueryOrFragment => "query_or_fragment",
            Self::PathNotAllowed => "path_not_allowed",
            Self::PathMismatch => "path_mismatch",
            Self::EnvironmentLocked => "environment_locked",
            Self::SaveAndReset => "save_and_reset",
            Self::PasskeyAcknowledgementRequired => "passkey_acknowledgement_required",
        }
    }
}

/// Why a public URL was rejected.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct PublicUrlError {
    pub code: PublicUrlErrorCode,
    pub message: String,
}

impl PublicUrlError {
    pub fn new(code: PublicUrlErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Validate a public URL: absolute http or https, a host, no credentials, no
/// query or fragment, and no wildcard host. Any path is accepted here.
pub fn parse_public_url(value: &str) -> Result<Url, PublicUrlError> {
    use PublicUrlErrorCode::*;
    let invalid = || {
        PublicUrlError::new(
            InvalidUrl,
            "the public URL must be an absolute http or https URL",
        )
    };
    let url = Url::parse(value.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid());
    }
    if url.host_str().is_some_and(|host| host.contains('*')) {
        return Err(PublicUrlError::new(
            WildcardHost,
            "the public URL must name a single host",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PublicUrlError::new(
            Credentials,
            "the public URL must not include a username or password",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(PublicUrlError::new(
            QueryOrFragment,
            "the public URL must not include a query or fragment",
        ));
    }
    Ok(url)
}

/// Validate a value an administrator saves and return the stored form. The
/// path must be empty or equal to the base path the instance serves under.
pub fn normalize_saved_public_url(value: &str, base_path: &str) -> Result<String, PublicUrlError> {
    let url = parse_public_url(value)?;
    let path = url.path().trim_end_matches('/');
    let base_path = base_path.trim_end_matches('/');
    if !path.is_empty() && path != base_path {
        return Err(if base_path.is_empty() {
            PublicUrlError::new(
                PublicUrlErrorCode::PathNotAllowed,
                "the public URL must not include a path because Scryer is served at the root",
            )
        } else {
            PublicUrlError::new(
                PublicUrlErrorCode::PathMismatch,
                format!("the public URL path must be empty or {base_path}"),
            )
        });
    }
    Ok(format!("{}{path}", url.origin().ascii_serialization()))
}

/// What saving a public URL would do to the running passkeys after the next
/// restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasskeyImpact {
    /// Passkeys do not come from the public URL (explicit WebAuthn variables,
    /// or passkeys are off), so the change cannot affect them.
    Unaffected,
    /// The relying party stays the same.
    Unchanged,
    /// Passkeys move to a different domain; existing passkeys stop working.
    Changed,
    /// The new value cannot carry passkeys; passkeys turn off.
    Disabled,
}

impl PasskeyImpact {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unaffected => "unaffected",
            Self::Unchanged => "unchanged",
            Self::Changed => "changed",
            Self::Disabled => "disabled",
        }
    }

    /// Whether existing passkeys would stop working.
    pub fn breaks_existing_passkeys(self) -> bool {
        matches!(self, Self::Changed | Self::Disabled)
    }
}

/// Compare the running relying party with the one a saved value would give
/// at the next start, using the same derivation startup uses. Returns the
/// impact and the next relying-party ID.
pub fn passkey_impact_of_saved_value(
    addressing: &InstanceAddressing,
    next_saved: Option<&str>,
) -> (PasskeyImpact, Option<String>) {
    if addressing.passkey_rp_source != PasskeyRelyingPartySource::PublicUrl {
        return (PasskeyImpact::Unaffected, None);
    }
    let next = next_saved
        .and_then(|value| parse_public_url(value).ok())
        .and_then(|url| passkey_relying_party_from_public_url(&url))
        .map(|(rp_id, _)| rp_id);
    match next {
        None => (PasskeyImpact::Disabled, None),
        Some(rp_id) if addressing.passkey_rp_id.as_deref() == Some(rp_id.as_str()) => {
            (PasskeyImpact::Unchanged, Some(rp_id))
        }
        Some(rp_id) => (PasskeyImpact::Changed, Some(rp_id)),
    }
}

/// A change that breaks passkeys needs explicit acknowledgement while anyone
/// has one, and also when the count could not be read.
pub fn passkey_acknowledgement_required(
    impact: PasskeyImpact,
    counts: Option<crate::PasskeyEnrollmentCounts>,
) -> bool {
    impact.breaks_existing_passkeys() && counts.is_none_or(|counts| counts.users_with_passkeys > 0)
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
    fn rejections_carry_stable_codes() {
        let code = |value: &str| parse_public_url(value).unwrap_err().code;
        assert_eq!(code("not a URL"), PublicUrlErrorCode::InvalidUrl);
        assert_eq!(code("ftp://media.home"), PublicUrlErrorCode::InvalidUrl);
        assert_eq!(code("https://*.home"), PublicUrlErrorCode::WildcardHost);
        assert_eq!(
            code("https://user:secret@media.home"),
            PublicUrlErrorCode::Credentials
        );
        assert_eq!(
            code("https://media.home?a=b"),
            PublicUrlErrorCode::QueryOrFragment
        );
        assert_eq!(
            normalize_saved_public_url("https://media.home/scryer", "")
                .unwrap_err()
                .code,
            PublicUrlErrorCode::PathNotAllowed
        );
        assert_eq!(
            normalize_saved_public_url("https://media.home/other", "/scryer")
                .unwrap_err()
                .code,
            PublicUrlErrorCode::PathMismatch
        );
    }

    fn addressing(source: PasskeyRelyingPartySource, rp_id: Option<&str>) -> InstanceAddressing {
        InstanceAddressing {
            passkey_rp_id: rp_id.map(str::to_string),
            passkey_rp_source: source,
            ..Default::default()
        }
    }

    #[test]
    fn passkey_impact_matrix() {
        use PasskeyImpact::*;
        let derived = addressing(PasskeyRelyingPartySource::PublicUrl, Some("media.home"));
        let impact = |addressing: &InstanceAddressing, next: Option<&str>| {
            passkey_impact_of_saved_value(addressing, next).0
        };
        // Same domain, even with a different port, path or letter case.
        assert_eq!(
            impact(&derived, Some("https://Media.Home:8443/scryer")),
            Unchanged
        );
        // A different domain moves the relying party.
        assert_eq!(impact(&derived, Some("https://other.home")), Changed);
        // Plain http on the same non-localhost domain, an IP literal, or
        // clearing the value leaves no relying party at all.
        assert_eq!(impact(&derived, Some("http://media.home")), Disabled);
        assert_eq!(impact(&derived, Some("https://192.168.1.20")), Disabled);
        assert_eq!(impact(&derived, None), Disabled);
        // Explicit WebAuthn variables, or passkeys already off, are unaffected.
        for unaffected in [
            addressing(PasskeyRelyingPartySource::Environment, Some("media.home")),
            addressing(PasskeyRelyingPartySource::None, None),
        ] {
            assert_eq!(impact(&unaffected, None), Unaffected);
            assert_eq!(impact(&unaffected, Some("https://other.home")), Unaffected);
        }
        assert_eq!(
            passkey_impact_of_saved_value(&derived, Some("https://other.home"))
                .1
                .as_deref(),
            Some("other.home")
        );
    }

    #[test]
    fn acknowledgement_is_required_only_when_registered_passkeys_would_break() {
        use crate::PasskeyEnrollmentCounts as Counts;
        let some = Some(Counts {
            users_with_passkeys: 2,
            passkey_only_users: 1,
        });
        let none_registered = Some(Counts::default());
        for impact in [PasskeyImpact::Changed, PasskeyImpact::Disabled] {
            assert!(passkey_acknowledgement_required(impact, some));
            // An unreadable count fails safe.
            assert!(passkey_acknowledgement_required(impact, None));
            assert!(!passkey_acknowledgement_required(impact, none_registered));
        }
        for impact in [PasskeyImpact::Unchanged, PasskeyImpact::Unaffected] {
            assert!(!passkey_acknowledgement_required(impact, some));
            assert!(!passkey_acknowledgement_required(impact, None));
        }
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
