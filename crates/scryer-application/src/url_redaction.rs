//! Credential redaction for indexer and tracker URLs.
//!
//! Torznab/Newznab download links carry the operator's indexer key in the
//! query string (`...&apikey=...` or Jackett's `jackett_apikey`), and private
//! trackers do the same with `passkey`, `rss_key`, `token`, `auth`, `authkey`
//! or `torrent_pass`. Feed and download URLs may also embed HTTP userinfo
//! (`https://user:pass@host/...`). The live URL is needed to fetch the
//! release, but every copy that is persisted for display, sent in a
//! notification, or written to a log must lose the credential first.

use std::sync::OnceLock;

use regex::{Captures, Regex};

/// Replacement written in place of a credential value.
pub const REDACTED_SECRET: &str = "[redacted]";

fn credential_query_param_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        // `\b` keeps names that merely end in a credential word (`oauth=`,
        // `x_token=`) out, while `-`/`_` separators and case are tolerated.
        // An already-redacted value is matched first so redaction is
        // idempotent instead of growing a stray `]` on each pass.
        Regex::new(
            r#"(?i)(?P<prefix>\b(?:api[_-]?key|pass[_-]?key|rss[_-]?key|token|auth|authkey|jackett_apikey|torrent_pass)=)(?P<value>\[redacted\]|[^&#;\s"'<>),\]}]+)"#,
        )
        .expect("credential query parameter regex should compile")
    })
}

fn url_userinfo_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        // The authority's userinfo runs from `scheme://` to the last `@` before
        // the path, query or fragment. An already-redacted userinfo matches
        // again and is rewritten to itself, so redaction stays idempotent.
        Regex::new(r#"(?i)(?P<scheme>\b[a-z][a-z0-9+.-]*://)(?P<userinfo>[^/?#\s"'<>]+)@"#)
            .expect("url userinfo regex should compile")
    })
}

/// Replace the value of every credential-bearing query parameter in `raw` with
/// [`REDACTED_SECRET`], and replace any URL userinfo (`user:pass@`) with
/// [`REDACTED_SECRET`], leaving the rest of the text (scheme, host, path, other
/// parameters, fragment) untouched. Text without such a parameter is returned
/// unchanged, so this is safe to run over free-form strings and values that
/// may already have been redacted.
pub fn redact_url_credentials(raw: &str) -> String {
    let without_userinfo = url_userinfo_regex().replace_all(raw, |captures: &Captures<'_>| {
        format!("{}{REDACTED_SECRET}@", &captures["scheme"])
    });
    credential_query_param_regex()
        .replace_all(&without_userinfo, |captures: &Captures<'_>| {
            format!("{}{REDACTED_SECRET}", &captures["prefix"])
        })
        .into_owned()
}

/// [`redact_url_credentials`] over an optional value.
pub fn redact_optional_url_credentials(value: Option<String>) -> Option<String> {
    value.map(|value| redact_url_credentials(&value))
}

/// A `Debug`/`Display` view of a URL that prints it with credentials removed.
/// Use it in log fields for a value that must keep its live credential.
pub struct RedactedUrl<'a>(pub &'a str);

impl std::fmt::Display for RedactedUrl<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&redact_url_credentials(self.0))
    }
}

impl std::fmt::Debug for RedactedUrl<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&redact_url_credentials(self.0), f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_every_credential_parameter_name() {
        for name in [
            "apikey",
            "api_key",
            "api-key",
            "passkey",
            "pass_key",
            "token",
            "rss_key",
            "rsskey",
            "rss-key",
            "auth",
            "authkey",
            "jackett_apikey",
            "torrent_pass",
        ] {
            let url = format!("https://indexer.invalid/api?t=get&{name}=s3cret&id=42");
            assert_eq!(
                redact_url_credentials(&url),
                format!("https://indexer.invalid/api?t=get&{name}=[redacted]&id=42"),
                "parameter {name}"
            );
        }
    }

    #[test]
    fn parameter_names_match_case_insensitively() {
        assert_eq!(
            redact_url_credentials(
                "https://indexer.invalid/api?ApiKey=abc&PASSKEY=def&Rss_Key=ghi"
            ),
            "https://indexer.invalid/api?ApiKey=[redacted]&PASSKEY=[redacted]&Rss_Key=[redacted]"
        );
    }

    #[test]
    fn redacts_tracker_and_jackett_keys_and_keeps_unrelated_parameters() {
        assert_eq!(
            redact_url_credentials(
                "https://tracker.invalid/dl?Jackett_ApiKey=aaa&AUTHKEY=bbb&Torrent_Pass=ccc&torrent_id=7&file=Paper+Lantern"
            ),
            "https://tracker.invalid/dl?Jackett_ApiKey=[redacted]&AUTHKEY=[redacted]&Torrent_Pass=[redacted]&torrent_id=7&file=Paper+Lantern"
        );
    }

    #[test]
    fn redacts_multiple_parameters_and_keeps_the_rest_of_the_url() {
        assert_eq!(
            redact_url_credentials(
                "https://tracker.invalid/dl/77.torrent?passkey=aaa&name=Paper+Lantern&token=bbb"
            ),
            "https://tracker.invalid/dl/77.torrent?passkey=[redacted]&name=Paper+Lantern&token=[redacted]"
        );
    }

    #[test]
    fn stops_the_value_at_a_fragment() {
        assert_eq!(
            redact_url_credentials("https://indexer.invalid/get?id=9&apikey=zzz#details"),
            "https://indexer.invalid/get?id=9&apikey=[redacted]#details"
        );
    }

    #[test]
    fn leaves_text_without_credentials_untouched() {
        for raw in [
            "Harbor.Lights.2031.1080p.WEB-DL-NOGRP",
            "https://indexer.invalid/get/harbor-lights.nzb",
            "magnet:?xt=urn:btih:abcdef&dn=Harbor+Lights",
            "https://indexer.invalid/api?oauth=keep&x_token=keep",
            "https://tracker.invalid/dl?torrent_passes=keep&x_authkey=keep",
            "weaver://job/job-1",
        ] {
            assert_eq!(redact_url_credentials(raw), raw);
        }
    }

    #[test]
    fn redacts_url_userinfo() {
        assert_eq!(
            redact_url_credentials("https://feeduser:s3cret@lists.invalid/feed.json?page=2"),
            "https://[redacted]@lists.invalid/feed.json?page=2"
        );
        assert_eq!(
            redact_url_credentials("see http://token-only@lists.invalid/rss and more"),
            "see http://[redacted]@lists.invalid/rss and more"
        );
        assert_eq!(
            redact_url_credentials("https://user:p@ss@lists.invalid/x?apikey=k"),
            "https://[redacted]@lists.invalid/x?apikey=[redacted]"
        );
    }

    #[test]
    fn leaves_at_signs_outside_the_authority_untouched() {
        for raw in [
            "https://lists.invalid/users/@someone/feed",
            "https://lists.invalid/feed?contact=a@b.invalid",
            "mailto:someone@lists.invalid",
            "someone@lists.invalid",
        ] {
            assert_eq!(redact_url_credentials(raw), raw);
        }
    }

    #[test]
    fn userinfo_redaction_is_idempotent() {
        let once = redact_url_credentials("https://u:p@lists.invalid/a");
        assert_eq!(redact_url_credentials(&once), once);
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact_url_credentials("https://indexer.invalid/api?apikey=abc&t=get");
        assert_eq!(redact_url_credentials(&once), once);
    }

    #[test]
    fn redacted_url_view_hides_the_key_in_display_and_debug() {
        let url = "https://indexer.invalid/api?t=get&apikey=live-key";
        assert!(!format!("{}", RedactedUrl(url)).contains("live-key"));
        assert!(!format!("{:?}", RedactedUrl(url)).contains("live-key"));
    }
}
