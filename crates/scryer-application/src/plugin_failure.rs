//! Stable names for the ways a plugin call can fail.
//!
//! A failed plugin call travels as [`AppError::PluginFailed`]: the host's
//! English sentence, plus a [`PluginFailure`] whose reason rides beside it as
//! `extensions.reason`, so a client can explain the failure in its own language
//! instead of showing or matching the sentence. A reason names the condition,
//! never the wording, and is API once released: reword a sentence freely, but
//! do not rename or reuse its reason.
//!
//! Most reasons mirror the code a plugin answered with. The rest name what the
//! host saw when the plugin never answered at all.

use scryer_plugin_sdk::{PluginError, PluginErrorCode};

use crate::AppError;
use crate::url_redaction::redact_url_credentials;

/// The longest guest sentence a failure carries to clients, in characters.
const DETAIL_MAX_CHARS: usize = 300;

/// Generic sentences plugin kits put on an error they know nothing about.
/// They add nothing beside a translated reason, so a failure carries no detail
/// for them.
const GENERIC_PUBLIC_MESSAGES: &[&str] = &[
    "indexer component failed",
    "the plugin could not complete the request",
    "the download client plugin could not complete the request",
];

/// The longest reason id a failure carries, in characters, `PLUGIN_` included.
const REASON_MAX_CHARS: usize = 64;

const REASON_PREFIX: &str = "PLUGIN_";

/// The open error codes the plugin kit shares, as their wire strings.
///
/// A plugin may send any code; these are the ones the host acts on beyond the
/// seven well-known `PluginErrorCode` variants. Each is API once a released
/// plugin sends it: never rename, reuse or repurpose one. Compare a received
/// code with `code.as_str() == plugin_codes::X`, or use the constant as a match
/// pattern on `code.as_str()`.
pub mod plugin_codes {
    /// The service rejected the configured username or password.
    pub const INVALID_USERNAME_PASSWORD: &str = "invalid_username_password";
    /// The service rejected the configured API key.
    pub const INVALID_API_KEY: &str = "invalid_api_key";
    /// The service rejected the configured passkey.
    pub const INVALID_PASSKEY: &str = "invalid_passkey";
    /// A configured cookie was rejected or has expired.
    pub const INVALID_COOKIE: &str = "invalid_cookie";
    /// The service demands a CAPTCHA before it will answer.
    pub const CAPTCHA_REQUIRED: &str = "captcha_required";
    /// The service demands a second authentication factor.
    pub const TWO_FACTOR_REQUIRED: &str = "two_factor_required";
    /// The service says the account is banned.
    pub const ACCOUNT_BANNED: &str = "account_banned";
    /// The service says the account is disabled, suspended or not activated.
    pub const ACCOUNT_DISABLED: &str = "account_disabled";
    /// The configured URL answers, but not with the expected API.
    pub const ENDPOINT_NOT_FOUND: &str = "endpoint_not_found";
    /// A secure (TLS) connection to the service could not be established.
    pub const TLS_ERROR: &str = "tls_error";
    /// The service answered with something the plugin cannot parse.
    pub const INVALID_RESPONSE: &str = "invalid_response";
    /// A daily or monthly allowance is used up; distinct from rate limiting.
    pub const QUOTA_EXCEEDED: &str = "quota_exceeded";
    /// The referenced download, torrent or list no longer exists.
    pub const ITEM_NOT_FOUND: &str = "item_not_found";
    /// The download client refused to add the item.
    pub const DOWNLOAD_REJECTED: &str = "download_rejected";
    /// The download client cannot write to the configured or requested path.
    pub const PATH_NOT_WRITABLE: &str = "path_not_writable";

    /// Every shared open code above, in declaration order.
    pub const ALL: &[&str] = &[
        INVALID_USERNAME_PASSWORD,
        INVALID_API_KEY,
        INVALID_PASSKEY,
        INVALID_COOKIE,
        CAPTCHA_REQUIRED,
        TWO_FACTOR_REQUIRED,
        ACCOUNT_BANNED,
        ACCOUNT_DISABLED,
        ENDPOINT_NOT_FOUND,
        TLS_ERROR,
        INVALID_RESPONSE,
        QUOTA_EXCEEDED,
        ITEM_NOT_FOUND,
        DOWNLOAD_REJECTED,
        PATH_NOT_WRITABLE,
    ];

    /// The codes that say the service refused the configured credentials or
    /// the account behind them.
    pub const CREDENTIAL_OR_ACCOUNT_REFUSALS: &[&str] = &[
        INVALID_USERNAME_PASSWORD,
        INVALID_API_KEY,
        INVALID_PASSKEY,
        INVALID_COOKIE,
        CAPTCHA_REQUIRED,
        TWO_FACTOR_REQUIRED,
        ACCOUNT_BANNED,
        ACCOUNT_DISABLED,
    ];
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PluginFailureReason {
    /// The plugin found its configuration unusable.
    InvalidConfig,
    /// The service the plugin talks to rejected its credentials.
    AuthFailed,
    /// The service asked the plugin to slow down.
    RateLimited,
    /// The service could not be reached or did not answer usefully.
    UpstreamUnavailable,
    /// The plugin does not offer what was asked of it.
    Unsupported,
    /// The plugin failed in a way it expects to clear up on its own.
    Temporary,
    /// The plugin failed in a way retrying will not fix.
    Permanent,
    /// The plugin crashed, or the host could not run it.
    HostFailure,
    /// The plugin did not finish within its time budget.
    TimedOut,
    /// The plugin exceeded the memory the host allows it.
    ResourceLimit,
    /// A code the plugin named beyond the well-known set, holding its reason
    /// id as [`PluginFailureReason::from_code`] derives it. Build it only
    /// through `from_code`, so an id that names a known reason is that reason.
    PluginCode(String),
}

impl PluginFailureReason {
    /// The reasons the host names on its own. Clients key translations on
    /// these, so renaming or removing one must be deliberate. A plugin can add
    /// more without a host change: see [`PluginFailureReason::from_code`].
    pub const KNOWN: &[Self] = &[
        Self::InvalidConfig,
        Self::AuthFailed,
        Self::RateLimited,
        Self::UpstreamUnavailable,
        Self::Unsupported,
        Self::Temporary,
        Self::Permanent,
        Self::HostFailure,
        Self::TimedOut,
        Self::ResourceLimit,
    ];

    pub fn as_str(&self) -> &str {
        match self {
            Self::InvalidConfig => "PLUGIN_INVALID_CONFIG",
            Self::AuthFailed => "PLUGIN_AUTH_FAILED",
            Self::RateLimited => "PLUGIN_RATE_LIMITED",
            Self::UpstreamUnavailable => "PLUGIN_UPSTREAM_UNAVAILABLE",
            Self::Unsupported => "PLUGIN_UNSUPPORTED",
            Self::Temporary => "PLUGIN_TEMPORARY_FAILURE",
            Self::Permanent => "PLUGIN_PERMANENT_FAILURE",
            Self::HostFailure => "PLUGIN_HOST_FAILURE",
            Self::TimedOut => "PLUGIN_TIMED_OUT",
            Self::ResourceLimit => "PLUGIN_RESOURCE_LIMIT",
            Self::PluginCode(id) => id,
        }
    }

    /// Whether this is one of the reasons in [`PluginFailureReason::KNOWN`].
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::PluginCode(_))
    }

    /// The reason for the code a plugin answered with.
    ///
    /// The seven well-known codes keep their fixed ids. Any other code becomes
    /// `PLUGIN_` plus the code upper-cased, so `invalid_username_password` is
    /// `PLUGIN_INVALID_USERNAME_PASSWORD`: every character outside `[A-Z0-9_]`
    /// becomes `_`, runs of `_` collapse to one, leading and trailing `_` are
    /// dropped, and the id is cut to [`REASON_MAX_CHARS`]. A code that leaves
    /// nothing is a permanent failure, and a code whose id is already a known
    /// reason is that reason.
    pub fn from_code(code: &PluginErrorCode) -> Self {
        match code {
            PluginErrorCode::InvalidConfig => return Self::InvalidConfig,
            PluginErrorCode::AuthFailed => return Self::AuthFailed,
            PluginErrorCode::RateLimited => return Self::RateLimited,
            PluginErrorCode::UpstreamUnavailable => return Self::UpstreamUnavailable,
            PluginErrorCode::Unsupported => return Self::Unsupported,
            PluginErrorCode::Temporary => return Self::Temporary,
            PluginErrorCode::Permanent => return Self::Permanent,
            PluginErrorCode::Other(_) => {}
        }
        let known = PluginErrorCode::from(code.as_str());
        if known.is_known() {
            return Self::from_code(&known);
        }

        let mut id = String::from(REASON_PREFIX);
        for character in code.as_str().chars() {
            let character = match character.to_ascii_uppercase() {
                upper @ ('A'..='Z' | '0'..='9') => upper,
                _ => '_',
            };
            if character == '_' && id.ends_with('_') {
                continue;
            }
            if id.len() == REASON_MAX_CHARS {
                break;
            }
            id.push(character);
        }
        let id = id.trim_end_matches('_');
        if id.len() < REASON_PREFIX.len() {
            return Self::Permanent;
        }
        Self::KNOWN
            .iter()
            .find(|reason| reason.as_str() == id)
            .cloned()
            .unwrap_or_else(|| Self::PluginCode(id.to_string()))
    }
}

/// Why a plugin call failed, in terms a client can translate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginFailure {
    pub reason: PluginFailureReason,
    /// The plugin's own public sentence, when it said something specific.
    /// Shown beside the translated reason, never used to choose it.
    pub detail: Option<String>,
}

impl PluginFailure {
    pub fn new(reason: PluginFailureReason) -> Self {
        Self {
            reason,
            detail: None,
        }
    }

    /// The failure a plugin reported. The public sentence becomes the detail,
    /// bounded and with any URL credential removed; the debug sentence never
    /// leaves the host.
    pub fn from_plugin_error(error: &PluginError) -> Self {
        Self {
            reason: PluginFailureReason::from_code(&error.code),
            detail: public_detail(&error.public_message),
        }
    }
}

fn public_detail(message: &str) -> Option<String> {
    let message = message.trim();
    if message.is_empty() || GENERIC_PUBLIC_MESSAGES.contains(&message) {
        return None;
    }
    let redacted = redact_url_credentials(message);
    let mut detail = redacted
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(DETAIL_MAX_CHARS + 1)
        .collect::<String>();
    if detail.chars().count() > DETAIL_MAX_CHARS {
        detail = detail.chars().take(DETAIL_MAX_CHARS - 1).collect();
        detail.push('…');
    }
    Some(detail)
}

impl AppError {
    /// A plugin answered `error` to the call `context` names. The sentence
    /// keeps the shape logs have always shown; the plugin's debug sentence is
    /// written to the debug log here and goes no further.
    pub fn plugin_failed(error: &PluginError, context: &str) -> Self {
        if let Some(debug_message) = error
            .debug_message
            .as_deref()
            .map(str::trim)
            .filter(|message| !message.is_empty())
        {
            tracing::debug!(
                context,
                code = ?error.code,
                public_message = %redact_url_credentials(&error.public_message),
                debug_message = %redact_url_credentials(debug_message),
                "plugin reported an error"
            );
        }
        Self::PluginFailed {
            message: format!(
                "{context}: plugin error {:?}: {}",
                error.code, error.public_message
            ),
            failure: PluginFailure::from_plugin_error(error),
        }
    }

    /// A plugin call that failed before the plugin could answer.
    pub fn plugin_host_failed(reason: PluginFailureReason, message: impl Into<String>) -> Self {
        Self::PluginFailed {
            message: message.into(),
            failure: PluginFailure::new(reason),
        }
    }

    /// Why a plugin call failed, if this error is one.
    pub fn plugin_failure(&self) -> Option<&PluginFailure> {
        match self {
            Self::PluginFailed { failure, .. } => Some(failure),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn plugin_error(code: PluginErrorCode, public_message: &str) -> PluginError {
        PluginError {
            code,
            public_message: public_message.to_string(),
            debug_message: Some("upstream said key=example-secret".to_string()),
            retry_after_seconds: None,
            details: None,
        }
    }

    #[test]
    fn every_reason_is_a_distinct_plugin_name_in_screaming_snake() {
        let distinct = PluginFailureReason::KNOWN
            .iter()
            .map(|reason| reason.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), PluginFailureReason::KNOWN.len());
        for reason in distinct {
            assert!(reason.starts_with("PLUGIN_"), "{reason}");
            assert!(
                reason
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{reason}"
            );
            assert!(!reason.ends_with('_') && !reason.contains("__"), "{reason}");
        }
    }

    #[test]
    fn every_plugin_code_names_its_own_reason() {
        let codes = [
            (PluginErrorCode::InvalidConfig, "PLUGIN_INVALID_CONFIG"),
            (PluginErrorCode::AuthFailed, "PLUGIN_AUTH_FAILED"),
            (PluginErrorCode::RateLimited, "PLUGIN_RATE_LIMITED"),
            (
                PluginErrorCode::UpstreamUnavailable,
                "PLUGIN_UPSTREAM_UNAVAILABLE",
            ),
            (PluginErrorCode::Unsupported, "PLUGIN_UNSUPPORTED"),
            (PluginErrorCode::Temporary, "PLUGIN_TEMPORARY_FAILURE"),
            (PluginErrorCode::Permanent, "PLUGIN_PERMANENT_FAILURE"),
        ];
        let mut seen = BTreeSet::new();
        for (code, expected) in codes {
            let reason = PluginFailureReason::from_code(&code);
            assert_eq!(reason.as_str(), expected, "{code:?}");
            assert!(reason.is_known(), "{code:?}");
            assert!(PluginFailureReason::KNOWN.contains(&reason));
            assert!(
                seen.insert(reason.as_str().to_string()),
                "{code:?} shares a reason"
            );

            // The same code arriving as its wire string names the same reason.
            let wire = serde_json::to_string(&code).expect("encode code");
            let decoded: PluginErrorCode = serde_json::from_str(&wire).expect("decode code");
            assert_eq!(PluginFailureReason::from_code(&decoded), reason, "{wire}");
            assert_eq!(
                PluginFailureReason::from_code(&PluginErrorCode::Other(code.as_str().into())),
                reason,
                "{wire}"
            );
        }
        assert_eq!(seen.len(), PluginErrorCode::KNOWN.len());
    }

    #[test]
    fn a_new_plugin_code_names_its_own_reason_end_to_end() {
        let error: PluginError = serde_json::from_str(
            r#"{"code":"invalid_username_password","public_message":"Login refused: check the username and password"}"#,
        )
        .expect("a plugin error with a new code decodes");
        assert_eq!(error.code.as_str(), "invalid_username_password");

        let failed = AppError::plugin_failed(&error, "indexer indexer_test component");
        let failure = failed.plugin_failure().expect("a plugin failure");
        assert_eq!(failure.reason.as_str(), "PLUGIN_INVALID_USERNAME_PASSWORD");
        assert!(!failure.reason.is_known());
        assert_eq!(
            failure.detail.as_deref(),
            Some("Login refused: check the username and password")
        );
        assert_eq!(
            failed.to_string(),
            "indexer indexer_test component: plugin error invalid_username_password: \
             Login refused: check the username and password"
        );
    }

    #[test]
    fn the_shared_plugin_codes_are_distinct_snake_case_and_unknown_to_the_sdk() {
        let distinct = plugin_codes::ALL.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), plugin_codes::ALL.len());
        for code in plugin_codes::ALL {
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{code}"
            );
            assert!(!PluginErrorCode::from(*code).is_known(), "{code}");
            let reason = PluginFailureReason::from_code(&PluginErrorCode::from(*code));
            assert_eq!(reason.as_str(), format!("PLUGIN_{}", code.to_uppercase()));
        }
        for code in plugin_codes::CREDENTIAL_OR_ACCOUNT_REFUSALS {
            assert!(plugin_codes::ALL.contains(code), "{code}");
        }
    }

    #[test]
    fn each_shared_plugin_code_names_the_reason_clients_translate() {
        // The open codes the plugin kit shares. Clients key a translation on
        // each reason id, so these ids must not drift.
        for (code, reason) in [
            (
                "invalid_username_password",
                "PLUGIN_INVALID_USERNAME_PASSWORD",
            ),
            ("invalid_api_key", "PLUGIN_INVALID_API_KEY"),
            ("invalid_passkey", "PLUGIN_INVALID_PASSKEY"),
            ("invalid_cookie", "PLUGIN_INVALID_COOKIE"),
            ("captcha_required", "PLUGIN_CAPTCHA_REQUIRED"),
            ("two_factor_required", "PLUGIN_TWO_FACTOR_REQUIRED"),
            ("account_banned", "PLUGIN_ACCOUNT_BANNED"),
            ("account_disabled", "PLUGIN_ACCOUNT_DISABLED"),
            ("endpoint_not_found", "PLUGIN_ENDPOINT_NOT_FOUND"),
            ("tls_error", "PLUGIN_TLS_ERROR"),
            ("invalid_response", "PLUGIN_INVALID_RESPONSE"),
            ("quota_exceeded", "PLUGIN_QUOTA_EXCEEDED"),
            ("item_not_found", "PLUGIN_ITEM_NOT_FOUND"),
            ("download_rejected", "PLUGIN_DOWNLOAD_REJECTED"),
            ("path_not_writable", "PLUGIN_PATH_NOT_WRITABLE"),
        ] {
            let failure = PluginFailureReason::from_code(&PluginErrorCode::from(code));
            assert_eq!(failure.as_str(), reason, "{code}");
            assert!(!failure.is_known(), "{code}");
        }
    }

    #[test]
    fn the_plugin_kits_generic_sentences_carry_no_detail() {
        for message in [
            "indexer component failed",
            "the plugin could not complete the request",
            "the download client plugin could not complete the request",
        ] {
            let error = PluginError {
                code: PluginErrorCode::from("invalid_api_key"),
                public_message: message.to_string(),
                debug_message: None,
                retry_after_seconds: None,
                details: None,
            };
            assert_eq!(
                PluginFailure::from_plugin_error(&error).detail,
                None,
                "{message}"
            );
        }
    }

    #[test]
    fn a_new_plugin_code_is_sanitised_into_a_bounded_reason_id() {
        let reason = |code: &str| PluginFailureReason::from_code(&PluginErrorCode::from(code));
        assert_eq!(reason("Quota-Exceeded").as_str(), "PLUGIN_QUOTA_EXCEEDED");
        assert_eq!(reason("__two__words__").as_str(), "PLUGIN_TWO_WORDS");
        assert_eq!(
            reason("captcha required!").as_str(),
            "PLUGIN_CAPTCHA_REQUIRED"
        );
        assert_eq!(reason("geo\u{e9}blocked").as_str(), "PLUGIN_GEO_BLOCKED");
        assert_eq!(reason("http_429").as_str(), "PLUGIN_HTTP_429");

        let long = reason(&"abc_".repeat(40));
        assert!(long.as_str().len() <= REASON_MAX_CHARS, "{}", long.as_str());
        assert!(
            long.as_str().starts_with("PLUGIN_ABC_ABC"),
            "{}",
            long.as_str()
        );
        assert!(!long.as_str().ends_with('_'), "{}", long.as_str());

        // Nothing usable left: a permanent failure, never a bare prefix.
        for empty in ["", "___", "!!!", "\u{e9}"] {
            assert_eq!(reason(empty), PluginFailureReason::Permanent, "{empty:?}");
        }

        // An id that already names a known reason is that reason.
        assert_eq!(reason("timed_out"), PluginFailureReason::TimedOut);
        assert_eq!(reason("AUTH_FAILED"), PluginFailureReason::AuthFailed);

        for code in ["Quota-Exceeded", "captcha required!", &"z".repeat(200)] {
            let id = reason(code);
            assert!(
                id.as_str()
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{}",
                id.as_str()
            );
            assert!(!id.as_str().contains("__"), "{}", id.as_str());
        }
    }

    #[test]
    fn a_plugin_error_keeps_its_sentence_and_names_its_reason() {
        let error = AppError::plugin_failed(
            &plugin_error(
                PluginErrorCode::AuthFailed,
                "Tracker login failed: bad password",
            ),
            "indexer indexer_search component",
        );
        assert_eq!(
            error.to_string(),
            "indexer indexer_search component: plugin error AuthFailed: \
             Tracker login failed: bad password"
        );
        let failure = error.plugin_failure().expect("a plugin failure");
        assert_eq!(failure.reason, PluginFailureReason::AuthFailed);
        assert_eq!(
            failure.detail.as_deref(),
            Some("Tracker login failed: bad password")
        );
        assert!(!format!("{error:?}").contains("example-secret"));
    }

    #[test]
    fn a_generic_or_blank_sentence_carries_no_detail() {
        for message in ["indexer component failed", "  ", ""] {
            let failure = PluginFailure::from_plugin_error(&plugin_error(
                PluginErrorCode::Temporary,
                message,
            ));
            assert_eq!(failure.reason, PluginFailureReason::Temporary);
            assert_eq!(failure.detail, None, "{message:?}");
        }
    }

    #[test]
    fn the_detail_is_bounded_single_line_and_free_of_url_credentials() {
        let failure = PluginFailure::from_plugin_error(&plugin_error(
            PluginErrorCode::UpstreamUnavailable,
            "fetch of https://indexer.example/api?apikey=abc123&t=caps failed\nretry later",
        ));
        let detail = failure.detail.expect("a detail");
        assert!(!detail.contains("abc123"), "{detail}");
        assert!(!detail.contains('\n'), "{detail}");

        let long = "x".repeat(DETAIL_MAX_CHARS * 2);
        let failure =
            PluginFailure::from_plugin_error(&plugin_error(PluginErrorCode::Permanent, &long));
        let detail = failure.detail.expect("a detail");
        assert_eq!(detail.chars().count(), DETAIL_MAX_CHARS);
        assert!(detail.ends_with('…'));
    }

    #[test]
    fn only_plugin_failures_name_one() {
        let host = AppError::plugin_host_failed(PluginFailureReason::TimedOut, "timed out");
        assert_eq!(
            host.plugin_failure(),
            Some(&PluginFailure::new(PluginFailureReason::TimedOut))
        );
        assert_eq!(host.to_string(), "timed out");
        assert_eq!(AppError::Repository("boom".into()).plugin_failure(), None);
    }
}
