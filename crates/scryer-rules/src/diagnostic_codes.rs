//! Stable rule validation conditions.
/// The source failed validation.
pub const RULE_VALIDATION_FAILED: &str = "RULE_VALIDATION_FAILED";
/// The rule could not be compiled.
pub const RULE_COMPILATION_ERROR: &str = "RULE_COMPILATION_ERROR";
/// The rule produced an invalid output.
pub const RULE_OUTPUT_INVALID: &str = "RULE_OUTPUT_INVALID";
/// The rule could not be evaluated.
pub const RULE_RUNTIME_ERROR: &str = "RULE_RUNTIME_ERROR";
/// The input path cannot be resolved statically.
pub const RULE_DYNAMIC_INPUT_PATH: &str = "RULE_DYNAMIC_INPUT_PATH";
/// The input path is outside the documented rule context.
pub const RULE_UNKNOWN_INPUT_PATH: &str = "RULE_UNKNOWN_INPUT_PATH";
/// The rule reads person-specific facts without permission.
pub const RULE_TARGETS_PERSON: &str = "RULE_TARGETS_PERSON";

/// The source must declare its assigned package.
pub const RULE_PACKAGE_REQUIRED: &str = "RULE_PACKAGE_REQUIRED";
/// Rule input cannot be imported into another name.
pub const RULE_INPUT_IMPORT_UNSUPPORTED: &str = "RULE_INPUT_IMPORT_UNSUPPORTED";
/// A maintenance matcher must define match.
pub const RULE_MATCH_REQUIRED: &str = "RULE_MATCH_REQUIRED";
/// A request matcher must define a vote or tags.
pub const RULE_VOTE_REQUIRED: &str = "RULE_VOTE_REQUIRED";
/// All supported diagnostic codes.
pub const ALL: &[&str] = &[
    RULE_PACKAGE_REQUIRED,
    RULE_INPUT_IMPORT_UNSUPPORTED,
    RULE_MATCH_REQUIRED,
    RULE_VOTE_REQUIRED,
    RULE_VALIDATION_FAILED,
    RULE_COMPILATION_ERROR,
    RULE_OUTPUT_INVALID,
    RULE_RUNTIME_ERROR,
    RULE_DYNAMIC_INPUT_PATH,
    RULE_UNKNOWN_INPUT_PATH,
    RULE_TARGETS_PERSON,
];
#[cfg(test)]
mod tests {
    #[test]
    fn codes_are_distinct_prefixed_identifiers() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(code.starts_with("RULE_"));
            assert!(code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'));
            assert!(seen.insert(*code));
        }
    }
}
