use chrono::{DateTime, Utc};
use reqwest::{Response, StatusCode, header::DATE};

const MAX_CLOCK_SKEW_SECONDS: u64 = 5 * 60;

pub(super) fn clock_skew_from_response(response: &Response, now: DateTime<Utc>) -> Option<bool> {
    clock_skew(
        response.status(),
        response.headers().get(DATE)?.to_str().ok()?,
        now,
    )
}

fn clock_skew(status: StatusCode, date: &str, now: DateTime<Utc>) -> Option<bool> {
    if status != StatusCode::UNAUTHORIZED {
        return None;
    }
    // The gateway's Date header is protocol evidence, independent of error wording.
    // Do not relabel unrelated 401s when Date is absent, malformed, or within the window.
    let server = DateTime::parse_from_rfc2822(date).ok()?;
    Some(
        now.signed_duration_since(server)
            .num_seconds()
            .unsigned_abs()
            > MAX_CLOCK_SKEW_SECONDS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_skew_requires_unauthorized_and_a_valid_server_date() {
        let now = "2026-10-30T00:34:17Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(clock_skew(StatusCode::UNAUTHORIZED, "invalid", now), None);
        assert_eq!(
            clock_skew(StatusCode::OK, "Fri, 30 Oct 2026 00:00:00 GMT", now),
            None
        );
        assert_eq!(
            clock_skew(StatusCode::FORBIDDEN, "Fri, 30 Oct 2026 00:00:00 GMT", now),
            None
        );
    }

    #[test]
    fn clock_skew_accepts_both_boundaries_and_detects_ahead_or_behind() {
        let server = "Fri, 30 Oct 2026 00:34:17 GMT";
        let base = "2026-10-30T00:34:17Z".parse::<DateTime<Utc>>().unwrap();
        for seconds in [-392, -301, -300, 0, 300, 301, 392] {
            let now = base + chrono::Duration::seconds(seconds);
            assert_eq!(
                clock_skew(StatusCode::UNAUTHORIZED, server, now),
                Some(seconds.unsigned_abs() > 300),
                "offset {seconds}"
            );
        }
    }
}
