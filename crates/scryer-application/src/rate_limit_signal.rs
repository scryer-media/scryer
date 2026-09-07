use std::time::Duration;

use crate::{AppError, RateLimitCooldownAction};
use scryer_outbound_http::OutboundHttpError;

/// Where a rate-limit signal was recognised. This enum is the compatibility
/// ledger for text-derived detection: the typed sources come first
/// (`AppTemporaryUnavailable`, `OutboundHttpRateLimited`); every variant after
/// them names one explicitly supported legacy phrase/format that
/// `RateLimitSignal::from_error` still parses out of an error message. Add a
/// text-derived source only by adding a variant here — and never for anything
/// but rate limiting (download-failover exhaustion, for one, is typed:
/// `AppError::DownloadSubmitFailoverExhausted`, and is not parsed from text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitSignalSource {
    AppTemporaryUnavailable,
    OutboundHttpRateLimited,
    RetryAfterText,
    RetryAfterSecondsText,
    RetryAfterHyphenText,
    RetryAfterUnderscoreText,
    RateLimitPhrase,
    TooManyRequestsPhrase,
    Http429Phrase,
    Status429Phrase,
    /// A newznab `<error code="500">` / `<error code="501">` document. These
    /// arrive on HTTP 200 with no `Retry-After` and no 429 anywhere, so the
    /// wall is invisible to every other source here.
    NewznabQuotaExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimitSignal {
    pub retry_after: Option<Duration>,
    pub source: RateLimitSignalSource,
    pub cooldown_action: RateLimitCooldownAction,
    pub status_code: Option<u16>,
    pub message: Option<String>,
}

impl RateLimitSignal {
    pub fn from_error(error: &AppError) -> Option<Self> {
        match error {
            AppError::NewznabQuotaExceeded {
                code: 500 | 501,
                message,
            } => Some(Self {
                retry_after: None,
                source: RateLimitSignalSource::NewznabQuotaExhausted,
                cooldown_action: RateLimitCooldownAction::RecordFallback,
                status_code: None,
                message: Some(message.clone()),
            }),
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                rate_limit_cooldown,
            } if *rate_limit_cooldown != RateLimitCooldownAction::None => Some(Self {
                retry_after: *retry_after,
                source: RateLimitSignalSource::AppTemporaryUnavailable,
                cooldown_action: *rate_limit_cooldown,
                status_code: status_code_from_text(message),
                message: Some(message.clone()),
            }),
            AppError::TemporaryUnavailable {
                rate_limit_cooldown: RateLimitCooldownAction::None,
                ..
            } => None,
            _ => Self::from_text(&error.to_string()),
        }
    }

    /// A newznab request/download limit reported as an error document.
    ///
    /// Indexers that publish no `apiCurrent`/`apiMax` announce the wall only
    /// this way, and they do it on HTTP 200, so nothing in [`Self::from_text`]
    /// can see it. Recognising it is what lets a spent account read as a rate
    /// limit rather than as an ordinary failure.
    ///
    /// The signal carries no `retry_after` on purpose. The document says
    /// nothing about when the allotment returns, and an indexer's reset is not
    /// necessarily UTC midnight, so inventing a deadline here would risk
    /// sitting out an account that came back hours earlier.
    /// [`Self::warrants_system_backoff`] is what decides the retry timing.
    ///
    /// Newznab 910 ("API disabled") is deliberately not included: providers
    /// reuse that slot for their own gate messages, so it stays a hard error.
    pub fn from_newznab_quota_message(message: &str) -> Option<Self> {
        let classified = crate::indexer_errors::classify_newznab_error_message(message)?;
        if !matches!(
            classified.classification,
            crate::types::IndexerErrorClassification::NewznabRequestLimitReached
                | crate::types::IndexerErrorClassification::NewznabDownloadLimitReached
        ) {
            return None;
        }
        Some(Self {
            retry_after: None,
            source: RateLimitSignalSource::NewznabQuotaExhausted,
            cooldown_action: RateLimitCooldownAction::RecordFallback,
            status_code: None,
            message: Some(message.to_string()),
        })
    }

    /// Whether the caller must also escalate the indexer's own system backoff
    /// instead of leaving the retry timing to the scheduler's cooldown.
    ///
    /// Most signals answer `false`, and correctly so: a 429 either carries a
    /// `Retry-After` or was already recorded by the transport, so its cooldown
    /// expires on the indexer's own schedule, and stacking a system backoff on
    /// top would disable an indexer over a transient limit.
    ///
    /// A newznab quota wall has neither. It is announced on HTTP 200 with no
    /// hint of when the allotment returns, so the scheduler can only fall back
    /// to a flat minute — which on the RSS path means a spent account is
    /// re-asked about sixty times an hour until the day rolls over. The
    /// escalating ladder is the only thing here that sizes that wait sensibly,
    /// and it self-heals: a success de-escalates it, so an account that comes
    /// back early is picked up again within the step it is already serving.
    pub fn warrants_system_backoff(&self) -> bool {
        self.retry_after.is_none()
            && matches!(self.source, RateLimitSignalSource::NewznabQuotaExhausted)
    }

    pub fn from_outbound_http_error(error: &OutboundHttpError) -> Option<Self> {
        match error {
            OutboundHttpError::DispatchRejected => None,
            OutboundHttpError::RateLimited(rate_limited) => Some(Self {
                retry_after: rate_limited.retry_after,
                source: RateLimitSignalSource::OutboundHttpRateLimited,
                cooldown_action: RateLimitCooldownAction::AlreadyRecorded,
                status_code: Some(429),
                message: Some(format!(
                    "outbound HTTP rate limited for {}",
                    rate_limited.request_label
                )),
            }),
            OutboundHttpError::Transport { .. } => None,
        }
    }

    pub fn from_text(message: &str) -> Option<Self> {
        if let Some((retry_after, source)) = retry_after_from_text(message) {
            return Some(Self {
                retry_after: Some(retry_after),
                source,
                cooldown_action: RateLimitCooldownAction::RecordFallback,
                status_code: status_code_from_text(message),
                message: Some(message.to_string()),
            });
        }

        let lower = message.to_ascii_lowercase();
        let source = if lower.contains("too many requests") {
            RateLimitSignalSource::TooManyRequestsPhrase
        } else if contains_status_marker(&lower, "http 429") {
            RateLimitSignalSource::Http429Phrase
        } else if contains_status_marker(&lower, "status 429") {
            RateLimitSignalSource::Status429Phrase
        } else if lower.contains("rate limit") || lower.contains("rate-limit") {
            RateLimitSignalSource::RateLimitPhrase
        } else {
            return None;
        };

        Some(Self {
            retry_after: None,
            source,
            cooldown_action: RateLimitCooldownAction::RecordFallback,
            status_code: status_code_from_text(message),
            message: Some(message.to_string()),
        })
    }
}

fn status_code_from_text(message: &str) -> Option<u16> {
    for marker in ["http ", "status "] {
        let lower = message.to_ascii_lowercase();
        for (index, _) in lower.match_indices(marker) {
            let digits = lower[index + marker.len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if let Ok(status) = digits.parse::<u16>() {
                return Some(status);
            }
        }
    }
    None
}

fn contains_status_marker(message: &str, marker: &str) -> bool {
    message.match_indices(marker).any(|(index, _)| {
        message[index + marker.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_digit())
    })
}

fn retry_after_from_text(message: &str) -> Option<(Duration, RateLimitSignalSource)> {
    let lower = message.to_ascii_lowercase();
    for (marker, source) in [
        (
            "retry_after_seconds=",
            RateLimitSignalSource::RetryAfterSecondsText,
        ),
        ("retry after", RateLimitSignalSource::RetryAfterText),
        ("retry-after", RateLimitSignalSource::RetryAfterHyphenText),
        (
            "retry_after",
            RateLimitSignalSource::RetryAfterUnderscoreText,
        ),
    ] {
        let Some(index) = lower.find(marker) else {
            continue;
        };
        let suffix = lower[index + marker.len()..].trim_start_matches([':', '=', ' ', '_']);
        let digits = suffix
            .chars()
            .skip_while(|ch| !ch.is_ascii_digit())
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if let Ok(seconds) = digits.parse::<u64>()
            && seconds > 0
        {
            return Some((Duration::from_secs(seconds), source));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_newznab_quotas_keep_code_without_inventing_a_reset_time() {
        for code in [500, 501] {
            let error = AppError::NewznabQuotaExceeded {
                code,
                message: "redacted provider message".into(),
            };
            let signal = RateLimitSignal::from_error(&error).unwrap();
            assert_eq!(signal.source, RateLimitSignalSource::NewznabQuotaExhausted);
            assert_eq!(signal.retry_after, None);
            assert!(signal.warrants_system_backoff());
        }
        assert!(
            RateLimitSignal::from_error(&AppError::Repository(
                "Newznab error 910: API disabled".into()
            ))
            .is_none()
        );
    }

    #[test]
    fn parses_retry_after_seconds_from_flattened_plugin_errors() {
        let signal = RateLimitSignal::from_text("HTTP 429: rate limited; retry_after_seconds=900")
            .expect("retry after should parse");
        assert_eq!(signal.retry_after, Some(Duration::from_secs(900)));
        assert_eq!(signal.source, RateLimitSignalSource::RetryAfterSecondsText);

        let signal = RateLimitSignal::from_text("Prowlarr rate limited (retry after 120s)")
            .expect("Prowlarr retry after should parse");
        assert_eq!(signal.retry_after, Some(Duration::from_secs(120)));
        assert_eq!(signal.source, RateLimitSignalSource::RetryAfterText);
    }

    #[test]
    fn detects_rate_limits_without_retry_after() {
        let signal =
            RateLimitSignal::from_text("HTTP 429: too many requests").expect("429 should parse");
        assert_eq!(signal.retry_after, None);
        assert_eq!(signal.status_code, Some(429));
        assert_eq!(signal.source, RateLimitSignalSource::TooManyRequestsPhrase);
    }

    #[test]
    fn does_not_match_bare_429_substrings() {
        assert!(RateLimitSignal::from_text("release title contains 429").is_none());
        assert!(RateLimitSignal::from_text("provider returned id 429001").is_none());
        assert!(RateLimitSignal::from_text("provider returned HTTP 429001").is_none());
    }

    #[test]
    fn preserves_typed_temporary_unavailable_retry_after() {
        let signal = RateLimitSignal::from_error(&AppError::rate_limited_temporary_unavailable(
            "provider temporarily unavailable",
            Some(Duration::from_secs(45)),
            RateLimitCooldownAction::RecordFallback,
        ))
        .expect("typed retry-after should be preserved");

        assert_eq!(signal.retry_after, Some(Duration::from_secs(45)));
        assert_eq!(
            signal.cooldown_action,
            RateLimitCooldownAction::RecordFallback
        );
        assert_eq!(
            signal.source,
            RateLimitSignalSource::AppTemporaryUnavailable
        );
    }

    #[test]
    fn plain_temporary_unavailable_is_not_a_rate_limit_signal() {
        assert!(
            RateLimitSignal::from_error(&AppError::temporary_unavailable(
                "provider temporarily unavailable",
                Some(Duration::from_secs(45)),
            ))
            .is_none()
        );
    }

    #[test]
    fn preserves_typed_outbound_http_retry_after() {
        let error = OutboundHttpError::RateLimited(scryer_outbound_http::RateLimitedError {
            scope: scryer_outbound_http::RateLimitScopeKey::from("plugin:artifact"),
            retry_after: Some(Duration::from_secs(75)),
            attempts: 1,
            retry_after_source: scryer_outbound_http::RetryAfterSource::Seconds,
            request_label: std::borrow::Cow::Borrowed("plugin artifact"),
        });
        let signal = RateLimitSignal::from_outbound_http_error(&error)
            .expect("typed outbound rate limit should be preserved");

        assert_eq!(signal.retry_after, Some(Duration::from_secs(75)));
        assert_eq!(signal.status_code, Some(429));
        assert_eq!(
            signal.cooldown_action,
            RateLimitCooldownAction::AlreadyRecorded
        );
        assert_eq!(
            signal.source,
            RateLimitSignalSource::OutboundHttpRateLimited
        );
    }
    #[test]
    fn a_newznab_request_limit_is_a_rate_limit_signal() {
        let signal =
            RateLimitSignal::from_newznab_quota_message("Newznab error 500: Request limit reached")
                .expect("a spent request quota must back the indexer off");
        assert_eq!(signal.source, RateLimitSignalSource::NewznabQuotaExhausted);
        assert_eq!(
            signal.cooldown_action,
            RateLimitCooldownAction::RecordFallback
        );
    }

    #[test]
    fn a_newznab_quota_wall_escalates_the_indexer_system_backoff() {
        let signal =
            RateLimitSignal::from_newznab_quota_message("Newznab error 500: Request limit reached")
                .expect("a spent request quota must back the indexer off");

        assert!(
            signal.warrants_system_backoff(),
            "the wall carries no Retry-After, so the flat fallback cooldown would re-ask a \
             spent account roughly sixty times an hour; the escalating ladder has to own the wait"
        );
    }

    #[test]
    fn a_typed_outbound_rate_limit_leaves_the_system_backoff_alone() {
        let error = OutboundHttpError::RateLimited(scryer_outbound_http::RateLimitedError {
            scope: scryer_outbound_http::RateLimitScopeKey::from("plugin:artifact"),
            retry_after: Some(Duration::from_secs(75)),
            attempts: 1,
            retry_after_source: scryer_outbound_http::RetryAfterSource::Seconds,
            request_label: std::borrow::Cow::Borrowed("plugin artifact"),
        });
        let signal = RateLimitSignal::from_outbound_http_error(&error)
            .expect("typed outbound rate limit should be preserved");

        assert!(
            !signal.warrants_system_backoff(),
            "a 429 that names its own Retry-After already expires on the indexer's schedule; \
             stacking a system backoff on top would disable an indexer over a transient limit"
        );
    }

    #[test]
    fn a_text_derived_retry_after_leaves_the_system_backoff_alone() {
        let signal = RateLimitSignal::from_text("rate limited, retry after 30 seconds")
            .expect("a retry-after phrase should still be recognised");

        assert!(
            signal.retry_after.is_some(),
            "the phrase carries its own delay"
        );
        assert!(
            !signal.warrants_system_backoff(),
            "a signal that knows when to come back does not need the ladder to guess"
        );
    }

    #[test]
    fn a_newznab_download_limit_is_a_rate_limit_signal() {
        assert_eq!(
            RateLimitSignal::from_newznab_quota_message(
                "Newznab error 501: Download limit reached",
            )
            .map(|signal| signal.source),
            Some(RateLimitSignalSource::NewznabQuotaExhausted)
        );
    }

    #[test]
    fn other_newznab_errors_are_not_quota_walls() {
        // 910 is reused by providers for their own gate messages, so it stays a
        // hard error rather than something the indexer waits out.
        assert!(
            RateLimitSignal::from_newznab_quota_message("Newznab error 910: newznab not enabled")
                .is_none()
        );
        assert!(
            RateLimitSignal::from_newznab_quota_message("Newznab error 100: Invalid API Key")
                .is_none()
        );
        assert!(RateLimitSignal::from_newznab_quota_message("connection refused").is_none());
    }
}
