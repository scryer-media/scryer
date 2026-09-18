//! Resolves the metadata gateway (SMG) GraphQL endpoint once at startup.
//!
//! The endpoint is configuration, not data: it comes from the environment, a
//! compiled-in default, or the local development fallback. Compose files
//! routinely quote the environment value, and those quotes used to survive into
//! the URL, which left every metadata operation failing with an opaque transport
//! error and nothing naming the setting at fault. Resolution here is forgiving
//! about the quoting an operator actually writes, but it says so in the log, and
//! it refuses to hand a value that is not an absolute http(s) URL downstream.

use url::Url;

use crate::SMG_GRAPHQL_URL;

pub(crate) const METADATA_GATEWAY_URL_ENV: &str = "SCRYER_METADATA_GATEWAY_GRAPHQL_URL";
/// The build-time variable `build.rs` reads to fill in the compiled-in default.
const METADATA_GATEWAY_BUILD_ENV: &str = "SCRYER_SMG_GRAPHQL_URL";
const LOCAL_FALLBACK_URL: &str = "http://127.0.0.1:8090/graphql";
/// Keeps a rejected value readable in a single log line.
const MAX_REPORTED_VALUE_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Source {
    Environment,
    CompiledIn,
}

impl Source {
    /// The setting whose name the operator can actually go and fix.
    fn setting(self) -> &'static str {
        match self {
            Self::Environment => METADATA_GATEWAY_URL_ENV,
            Self::CompiledIn => METADATA_GATEWAY_BUILD_ENV,
        }
    }
}

/// The single metadata gateway endpoint this process uses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MetadataGatewayUrl {
    value: String,
}

impl MetadataGatewayUrl {
    pub(crate) fn from_env() -> Self {
        let explicit = std::env::var(METADATA_GATEWAY_URL_ENV).ok();
        Self::resolve(explicit.as_deref(), SMG_GRAPHQL_URL)
    }

    /// Resolution order: the explicit environment value, the compiled-in
    /// default, then the local development fallback.
    ///
    /// A setting that is *set but unusable* stops the chain instead of falling
    /// through: an operator who pointed the process at their own gateway and
    /// mistyped it gets the local fallback that refuses the connection loudly,
    /// not the official gateway answering quietly in its place.
    pub(crate) fn resolve(explicit: Option<&str>, compiled_in: Option<&str>) -> Self {
        if let Some(raw) = explicit.filter(|value| !value.trim().is_empty()) {
            return Self::from_setting(raw, Source::Environment)
                .unwrap_or_else(Self::local_fallback);
        }

        compiled_in
            .and_then(|value| Self::from_setting(value, Source::CompiledIn))
            .unwrap_or_else(Self::local_fallback)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.value
    }

    pub(crate) fn into_string(self) -> String {
        self.value
    }

    fn local_fallback() -> Self {
        Self {
            value: LOCAL_FALLBACK_URL.to_string(),
        }
    }

    /// An unset or empty setting is not a configuration error — it means "use
    /// the next source". A value that is set but unusable is named in the log
    /// before it is dropped, because the alternative is what issue #166 hit: an
    /// operator watching metadata fail with no way to tell which setting did it.
    fn from_setting(raw: &str, source: Source) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        let unquoted = strip_surrounding_quotes(trimmed).trim();
        if unquoted.is_empty() {
            return None;
        }
        if unquoted.len() != trimmed.len() {
            tracing::warn!(
                setting = source.setting(),
                value = %summarize(unquoted),
                "metadata gateway URL is wrapped in quotes; using the unquoted value"
            );
        }

        match validated(unquoted) {
            Some(value) => Some(Self { value }),
            None => {
                tracing::error!(
                    setting = source.setting(),
                    value = %summarize(unquoted),
                    "metadata gateway URL is not an absolute http(s) URL; ignoring it"
                );
                None
            }
        }
    }
}

/// Returns the operator's own string when it is an absolute http(s) URL, so a
/// working configuration reaches the gateway client byte for byte as before.
fn validated(candidate: &str) -> Option<String> {
    let url = Url::parse(candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    Some(candidate.to_string())
}

/// Removes exactly one matching pair of surrounding quotes, which is the shape
/// `KEY="value"` produces once the shell has handed the value over.
fn strip_surrounding_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() < 2 {
        return value;
    }

    let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
    let matched_pair = (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'');
    if matched_pair {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn summarize(value: &str) -> String {
    if value.chars().count() <= MAX_REPORTED_VALUE_CHARS {
        return value.to_string();
    }

    let mut truncated = value
        .chars()
        .take(MAX_REPORTED_VALUE_CHARS)
        .collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::LOCAL_FALLBACK_URL;
    use super::MetadataGatewayUrl;

    const GATEWAY: &str = "https://smg.example.test/graphql";

    #[test]
    fn keeps_a_bare_environment_value() {
        assert_eq!(
            MetadataGatewayUrl::resolve(Some(GATEWAY), None).as_str(),
            GATEWAY
        );
    }

    #[test]
    fn strips_surrounding_double_quotes() {
        assert_eq!(
            MetadataGatewayUrl::resolve(Some(&format!("\"{GATEWAY}\"")), None).as_str(),
            GATEWAY
        );
    }

    #[test]
    fn strips_surrounding_single_quotes() {
        assert_eq!(
            MetadataGatewayUrl::resolve(Some(&format!("'{GATEWAY}'")), None).as_str(),
            GATEWAY
        );
    }

    #[test]
    fn trims_whitespace_around_and_inside_the_quotes() {
        let quoted = format!("  \"  {GATEWAY}  \"  ");
        assert_eq!(
            MetadataGatewayUrl::resolve(Some(&quoted), None).as_str(),
            GATEWAY
        );
    }

    #[test]
    fn strips_quotes_from_the_compiled_in_default_too() {
        let quoted = format!("\"{GATEWAY}\"");
        assert_eq!(
            MetadataGatewayUrl::resolve(None, Some(&quoted)).as_str(),
            GATEWAY
        );
    }

    #[test]
    fn unset_environment_value_uses_the_compiled_in_default() {
        let compiled_in = "http://127.0.0.1:9000/graphql";
        assert_eq!(
            MetadataGatewayUrl::resolve(None, Some(compiled_in)).as_str(),
            compiled_in
        );
        assert_eq!(
            MetadataGatewayUrl::resolve(Some("   "), Some(compiled_in)).as_str(),
            compiled_in
        );
    }

    #[test]
    fn empty_environment_value_falls_back_to_the_local_default() {
        assert_eq!(
            MetadataGatewayUrl::resolve(Some(""), None).as_str(),
            LOCAL_FALLBACK_URL
        );
        assert_eq!(
            MetadataGatewayUrl::resolve(Some("\"\""), None).as_str(),
            LOCAL_FALLBACK_URL
        );
    }

    #[test]
    fn relative_value_falls_back_instead_of_panicking() {
        assert_eq!(
            MetadataGatewayUrl::resolve(Some("smg.example.test/graphql"), None).as_str(),
            LOCAL_FALLBACK_URL
        );
    }

    #[test]
    fn non_http_scheme_falls_back() {
        assert_eq!(
            MetadataGatewayUrl::resolve(Some("file:///tmp/graphql"), None).as_str(),
            LOCAL_FALLBACK_URL
        );
    }

    #[test]
    fn unmatched_quotes_are_left_alone_and_rejected() {
        let mismatched = format!("\"{GATEWAY}'");
        assert_eq!(
            MetadataGatewayUrl::resolve(Some(&mismatched), None).as_str(),
            LOCAL_FALLBACK_URL
        );
    }

    #[test]
    fn text_after_a_closing_quote_is_not_stripped() {
        let trailing = format!("\"{GATEWAY}\"extra");
        assert_eq!(
            MetadataGatewayUrl::resolve(Some(&trailing), None).as_str(),
            LOCAL_FALLBACK_URL
        );
    }

    #[test]
    fn invalid_environment_value_does_not_fall_through_to_the_compiled_in_default() {
        let compiled_in = "http://127.0.0.1:9000/graphql";
        assert_eq!(
            MetadataGatewayUrl::resolve(Some("not a url"), Some(compiled_in)).as_str(),
            LOCAL_FALLBACK_URL
        );
    }

    #[test]
    fn summarises_a_long_rejected_value_without_panicking() {
        let long_value = "x".repeat(400);
        assert_eq!(
            MetadataGatewayUrl::resolve(Some(&long_value), None).as_str(),
            LOCAL_FALLBACK_URL
        );
    }
}
