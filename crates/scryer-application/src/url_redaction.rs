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

/// Every query parameter that carries a credential, lowercased with `_` and
/// `-` removed. The single source for both the redactor and the stored
/// release-attempt hint.
const CREDENTIAL_PARAMETER_NAMES: &[&str] = &[
    "apikey",
    "apiaccess",
    "token",
    "auth",
    "password",
    "passkey",
    "jackettapikey",
    "rsskey",
    "authkey",
    "torrentpass",
];

/// Whether a query parameter name carries a credential. Case and `_`/`-`
/// separators are ignored; a name that merely contains a credential word
/// (`oauth`, `x_token`, `torrent_passes`) does not.
pub fn is_credential_parameter_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase().replace(['_', '-'], "");
    CREDENTIAL_PARAMETER_NAMES.contains(&normalized.as_str())
}

fn query_param_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        // The whole parameter name is captured and checked against the list,
        // so a name that only ends in a credential word is kept. An
        // already-redacted value is matched first so redaction is idempotent
        // instead of growing a stray `]` on each pass.
        Regex::new(
            r#"(?P<prefix>\b(?P<name>[A-Za-z0-9_-]+)=)(?P<value>\[redacted\]|[^&#;\s"'<>),\]}]+)"#,
        )
        .expect("query parameter regex should compile")
    })
}

fn encoded_query_param_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        // A URL carried percent-encoded inside another one, such as a magnet
        // link's `tr=` announce URL: `%3F`/`%26` start a parameter, `%3D`
        // ends its name, and the value runs to the next `&` or `%26`.
        Regex::new(
            r#"(?i)(?P<prefix>(?:%3F|%26)(?P<name>[A-Za-z0-9_-]+)%3D)(?P<value>%5Bredacted%5D|(?:[^&#;%\s"'<>),\]}]|%(?:[013-9a-f][0-9a-f]|2[0-57-9a-f]))+)"#,
        )
        .expect("encoded query parameter regex should compile")
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
    let plain = redact_plain_query_params(&without_userinfo);
    encoded_query_param_regex()
        .replace_all(&plain, |captures: &Captures<'_>| {
            if is_credential_parameter_name(&captures["name"]) {
                format!("{}%5Bredacted%5D", &captures["prefix"])
            } else {
                captures[0].to_string()
            }
        })
        .into_owned()
}

fn redact_plain_query_params(raw: &str) -> String {
    query_param_regex()
        .replace_all(raw, |captures: &Captures<'_>| {
            if is_credential_parameter_name(&captures["name"]) {
                format!("{}{REDACTED_SECRET}", &captures["prefix"])
            } else {
                // A kept value can itself be a URL with credentials, such as
                // a magnet's unencoded `tr=` announce URL.
                format!(
                    "{}{}",
                    &captures["prefix"],
                    redact_plain_query_params(&captures["value"])
                )
            }
        })
        .into_owned()
}

/// Remove credentials from a domain event before it is stored, so the event
/// log never holds a live indexer or tracker key.
pub(crate) fn redact_new_domain_event(event: &mut scryer_domain::NewDomainEvent) {
    if let scryer_domain::DomainEventPayload::ReleaseGrabbed(data) = &mut event.payload {
        data.source_hint = redact_optional_url_credentials(data.source_hint.take());
    }
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
            "apiaccess",
            "api_access",
            "password",
            "Password",
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
    fn redacts_credentials_of_a_magnet_announce_url() {
        assert_eq!(
            redact_url_credentials(
                "magnet:?xt=urn:btih:abcdef&dn=Harbor+Lights&tr=https%3A%2F%2Ftracker.invalid%2Fannounce%3Fpasskey%3Ds3cret%26torrent_id%3D7"
            ),
            "magnet:?xt=urn:btih:abcdef&dn=Harbor+Lights&tr=https%3A%2F%2Ftracker.invalid%2Fannounce%3Fpasskey%3D%5Bredacted%5D%26torrent_id%3D7"
        );
        assert_eq!(
            redact_url_credentials(
                "magnet:?xt=urn:btih:abcdef&tr=https://tracker.invalid/announce?authkey=s3cret"
            ),
            "magnet:?xt=urn:btih:abcdef&tr=https://tracker.invalid/announce?authkey=[redacted]"
        );
        let once = redact_url_credentials(
            "magnet:?xt=urn:btih:abcdef&tr=http%3A%2F%2Ft.invalid%2Fa%3Fpasskey%3Dk",
        );
        assert_eq!(redact_url_credentials(&once), once);
    }

    #[test]
    fn credential_parameter_names_are_one_list() {
        for name in [
            "ApiKey",
            "api-access",
            "PASSWORD",
            "torrent_pass",
            "Rss_Key",
        ] {
            assert!(is_credential_parameter_name(name), "{name}");
        }
        for name in ["oauth", "x_token", "torrent_passes", "x_authkey", "id"] {
            assert!(!is_credential_parameter_name(name), "{name}");
        }
    }

    #[test]
    fn a_grab_event_loses_its_indexer_key_before_it_is_stored() {
        use scryer_domain::{
            DomainEventActorKind, DomainEventPayload, DomainEventStream, DomainExternalIds,
            MediaFacet, NewDomainEvent, ReleaseGrabbedEventData, TitleContextSnapshot,
        };
        let mut event = NewDomainEvent {
            event_id: "event-1".to_string(),
            occurred_at: chrono::Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "system".to_string(),
            title_id: Some("title-1".to_string()),
            facet: Some(MediaFacet::Movie),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: "title-1".to_string(),
            },
            payload: DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: TitleContextSnapshot {
                    title_name: "Harbor Lights".to_string(),
                    facet: MediaFacet::Movie,
                    external_ids: DomainExternalIds::default(),
                    poster_url: None,
                    year: None,
                },
                source_title: Some("Harbor.Lights.2031.1080p.WEB-DL-NOGRP".to_string()),
                source_hint: Some(
                    "https://indexer.invalid/api?t=get&id=9&apiaccess=s3cret".to_string(),
                ),
                source_provider: None,
                download_id: None,
                episode_ids: Vec::new(),
                release_facts: None,
            }),
        };
        redact_new_domain_event(&mut event);
        let DomainEventPayload::ReleaseGrabbed(data) = event.payload else {
            panic!("payload kind should not change");
        };
        assert_eq!(
            data.source_hint.as_deref(),
            Some("https://indexer.invalid/api?t=get&id=9&apiaccess=[redacted]")
        );
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
