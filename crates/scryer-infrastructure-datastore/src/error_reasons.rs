//! Stable reasons for conditions raised at this infrastructure boundary.

/// database migration check failed; pending PostgreSQL migrations: {}
pub const DATASTORE_DATABASE_MIGRATION_CHECK_FAILED: &str =
    "DATASTORE_DATABASE_MIGRATION_CHECK_FAILED";

#[cfg(test)]
const ALL: &[&str] = &[DATASTORE_DATABASE_MIGRATION_CHECK_FAILED];

#[cfg(test)]
mod tests {
    #[test]
    fn reason_codes_are_distinct_and_stable_format() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(seen.insert(*code), "duplicate reason: {code}");
            assert!(["DATASTORE_"].iter().any(|prefix| code.starts_with(prefix)));
            assert!(
                code.bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            );
        }
    }
}
