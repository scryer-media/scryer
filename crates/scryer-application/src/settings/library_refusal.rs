//! Stable reasons for refused library settings, carried beside the message.
//!
//! Reason names describe conditions and are API once released. The normal
//! monitoring picker only submits supported choices; invalid values can still
//! arrive through API clients and must not require matching the message.

/// The requested default monitoring policy is unsupported for the library's facet.
pub const DEFAULT_MONITOR_TYPE_INVALID: &str = "LIBRARY_DEFAULT_MONITOR_TYPE_INVALID";

#[cfg(test)]
pub const ALL: &[&str] = &[DEFAULT_MONITOR_TYPE_INVALID];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn library_default_monitor_type_refusal_reasons_are_distinct_and_namespaced() {
        let distinct = ALL.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), ALL.len(), "a reason is listed twice");
        for reason in ALL {
            assert!(reason.starts_with("LIBRARY_"), "{reason}");
            assert!(
                reason
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{reason}"
            );
            assert!(!reason.ends_with('_') && !reason.contains("__"), "{reason}");
        }
    }
}
