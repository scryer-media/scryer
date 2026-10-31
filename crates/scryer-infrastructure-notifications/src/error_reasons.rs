//! Stable reasons for conditions raised at this infrastructure boundary.

/// notification channel {id}
pub const NOTIFICATION_CHANNEL_NOT_FOUND: &str = "NOTIFICATION_CHANNEL_NOT_FOUND";

/// notification subscription {id}
pub const NOTIFICATION_SUBSCRIPTION_NOT_FOUND: &str = "NOTIFICATION_SUBSCRIPTION_NOT_FOUND";

#[cfg(test)]
const ALL: &[&str] = &[
    NOTIFICATION_CHANNEL_NOT_FOUND,
    NOTIFICATION_SUBSCRIPTION_NOT_FOUND,
];

#[cfg(test)]
mod tests {
    #[test]
    fn reason_codes_are_distinct_and_stable_format() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(seen.insert(*code), "duplicate reason: {code}");
            assert!(
                ["NOTIFICATION_"]
                    .iter()
                    .any(|prefix| code.starts_with(prefix))
            );
            assert!(
                code.bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            );
        }
    }
}
