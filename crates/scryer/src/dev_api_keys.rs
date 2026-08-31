use chrono::{DateTime, Utc};
use scryer_application::{AppUseCase, DevelopmentApiKeySeed};

const DEV_API_KEYS_ENV: &str = "SCRYER_DEV_API_KEYS";

pub async fn sync_from_env(app: &AppUseCase) -> Result<(), String> {
    let value = std::env::var(DEV_API_KEYS_ENV).unwrap_or_default();
    let seeds = parse_declarations(&value)?;
    app.sync_development_api_keys(seeds)
        .await
        .map_err(|error| format!("invalid {DEV_API_KEYS_ENV} declaration: {error}"))
}

fn parse_declarations(value: &str) -> Result<Vec<DevelopmentApiKeySeed>, String> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }

    value
        .split(';')
        .enumerate()
        .map(|(index, entry)| parse_declaration(entry, index + 1))
        .collect()
}

fn parse_declaration(entry: &str, position: usize) -> Result<DevelopmentApiKeySeed, String> {
    let mut fields = entry.split('|');
    let username = fields.next().unwrap_or_default();
    let label = fields.next().unwrap_or_default();
    let raw_key = fields.next().unwrap_or_default();
    let expires_at = fields.next().unwrap_or_default();
    if username.is_empty()
        || label.is_empty()
        || raw_key.is_empty()
        || expires_at.is_empty()
        || fields.next().is_some()
    {
        return Err(format!(
            "entry {position} must be username|label|raw_key|expires_at"
        ));
    }

    let expires_at = if expires_at == "never" {
        None
    } else {
        let parsed = DateTime::parse_from_rfc3339(expires_at)
            .map_err(|_| format!("entry {position} has an invalid expiry"))?;
        if parsed.offset().local_minus_utc() != 0 {
            return Err(format!("entry {position} expiry must use UTC"));
        }
        Some(parsed.with_timezone(&Utc))
    };

    Ok(DevelopmentApiKeySeed {
        username: username.to_string(),
        label: label.to_string(),
        raw_key: raw_key.to_string(),
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_declarations;

    #[test]
    fn parses_never_and_utc_expiries() {
        let entries = parse_declarations(
            "admin|local|ska_AAAAAAAAAAAAAAAAAAAAAA.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA|never;admin|expires|ska_AQEBAQEBAQEBAQEBAQEBAQ.AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE|2030-01-02T03:04:05Z",
        )
        .expect("valid declarations");

        assert_eq!(entries.len(), 2);
        assert!(entries[0].expires_at.is_none());
        assert_eq!(
            entries[1].expires_at.expect("expiry").to_rfc3339(),
            "2030-01-02T03:04:05+00:00"
        );
    }

    #[test]
    fn rejects_malformed_entries_without_echoing_the_key() {
        let error = parse_declarations("admin|local|secret").expect_err("must fail");
        assert!(error.contains("entry 1"));
        assert!(!error.contains("secret"));
    }
}
