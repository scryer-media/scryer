//! Stable reasons for conditions raised at this infrastructure boundary.

/// accepted lifecycle job receipts require a real job run id
pub const CONFIGURATION_LIFECYCLE_JOB_RUN_ID_REQUIRED: &str =
    "CONFIGURATION_LIFECYCLE_JOB_RUN_ID_REQUIRED";

/// ambiguous setting key {}.{} found in categories: {}
pub const CONFIGURATION_SETTING_KEY_AMBIGUOUS: &str = "CONFIGURATION_SETTING_KEY_AMBIGUOUS";

/// a candidate transition must name the states it expects
pub const CONFIGURATION_CANDIDATE_EXPECTED_STATES_REQUIRED: &str =
    "CONFIGURATION_CANDIDATE_EXPECTED_STATES_REQUIRED";

/// copied rule sets must be ordinary user rules
pub const CONFIGURATION_RULE_COPY_SOURCE_NOT_USER_RULE: &str =
    "CONFIGURATION_RULE_COPY_SOURCE_NOT_USER_RULE";

/// failed lifecycle sequence marker does not preserve its step prefix
pub const CONFIGURATION_LIFECYCLE_STEP_PREFIX_MISMATCH: &str =
    "CONFIGURATION_LIFECYCLE_STEP_PREFIX_MISMATCH";

/// failed lifecycle sequence marker lacks its failed step id
pub const CONFIGURATION_LIFECYCLE_FAILED_STEP_ID_REQUIRED: &str =
    "CONFIGURATION_LIFECYCLE_FAILED_STEP_ID_REQUIRED";

/// a generic lifecycle job dispatch must begin in dispatching state
pub const CONFIGURATION_LIFECYCLE_DISPATCH_STATE_INVALID: &str =
    "CONFIGURATION_LIFECYCLE_DISPATCH_STATE_INVALID";

/// at least one subtitle provider config field must be provided
pub const CONFIGURATION_SUBTITLE_PROVIDER_UPDATE_EMPTY: &str =
    "CONFIGURATION_SUBTITLE_PROVIDER_UPDATE_EMPTY";

/// a lifecycle action step claim has an invalid target identity
pub const CONFIGURATION_LIFECYCLE_STEP_TARGET_INVALID: &str =
    "CONFIGURATION_LIFECYCLE_STEP_TARGET_INVALID";

/// a lifecycle action step claim must start a non-negative running attempt
pub const CONFIGURATION_LIFECYCLE_STEP_ATTEMPT_INVALID: &str =
    "CONFIGURATION_LIFECYCLE_STEP_ATTEMPT_INVALID";

/// a lifecycle action step finish must carry an attempt result
pub const CONFIGURATION_LIFECYCLE_ATTEMPT_RESULT_REQUIRED: &str =
    "CONFIGURATION_LIFECYCLE_ATTEMPT_RESULT_REQUIRED";

/// lifecycle action step does not match its persisted sequence identity
pub const CONFIGURATION_LIFECYCLE_SEQUENCE_IDENTITY_MISMATCH: &str =
    "CONFIGURATION_LIFECYCLE_SEQUENCE_IDENTITY_MISMATCH";

/// a lifecycle job receipt transition must name expected states
pub const CONFIGURATION_LIFECYCLE_RECEIPT_EXPECTED_STATES_REQUIRED: &str =
    "CONFIGURATION_LIFECYCLE_RECEIPT_EXPECTED_STATES_REQUIRED";

/// lifecycle rule {rule_set_id} already has an active candidate for title {title_id}
pub const CONFIGURATION_LIFECYCLE_CANDIDATE_ALREADY_ACTIVE: &str =
    "CONFIGURATION_LIFECYCLE_CANDIDATE_ALREADY_ACTIVE";

/// This lifecycle rule has live candidates or action history; disable it instead.
pub const CONFIGURATION_LIFECYCLE_RULE_HAS_HISTORY: &str =
    "CONFIGURATION_LIFECYCLE_RULE_HAS_HISTORY";

/// lifecycle sequence membership is terminal until a confirmed no-match
pub const CONFIGURATION_LIFECYCLE_MEMBERSHIP_TERMINAL: &str =
    "CONFIGURATION_LIFECYCLE_MEMBERSHIP_TERMINAL";

/// lifecycle sequence progress accepts at most 100 candidates per page
pub const CONFIGURATION_LIFECYCLE_CANDIDATE_PAGE_TOO_LARGE: &str =
    "CONFIGURATION_LIFECYCLE_CANDIDATE_PAGE_TOO_LARGE";

/// lifecycle sequence receipt projection accepts at most 700 steps
pub const CONFIGURATION_LIFECYCLE_RECEIPT_PAGE_TOO_LARGE: &str =
    "CONFIGURATION_LIFECYCLE_RECEIPT_PAGE_TOO_LARGE";

/// lifecycle sequence terminal marker and candidate terminal state disagree
pub const CONFIGURATION_LIFECYCLE_TERMINAL_STATE_MISMATCH: &str =
    "CONFIGURATION_LIFECYCLE_TERMINAL_STATE_MISMATCH";

/// lifecycle sequence terminal marker has invalid expected step ids
pub const CONFIGURATION_LIFECYCLE_TERMINAL_STEP_IDS_INVALID: &str =
    "CONFIGURATION_LIFECYCLE_TERMINAL_STEP_IDS_INVALID";

/// lifecycle terminal marker does not match persisted step evidence
pub const CONFIGURATION_LIFECYCLE_TERMINAL_EVIDENCE_MISMATCH: &str =
    "CONFIGURATION_LIFECYCLE_TERMINAL_EVIDENCE_MISMATCH";

/// plugin is no longer installed
pub const CONFIGURATION_PLUGIN_NOT_INSTALLED: &str = "CONFIGURATION_PLUGIN_NOT_INSTALLED";

/// plugin settings require an installed plugin
pub const CONFIGURATION_PLUGIN_SETTINGS_PLUGIN_REQUIRED: &str =
    "CONFIGURATION_PLUGIN_SETTINGS_PLUGIN_REQUIRED";

/// a retried lifecycle action step must advance its attempt
pub const CONFIGURATION_LIFECYCLE_RETRY_ATTEMPT_NOT_ADVANCED: &str =
    "CONFIGURATION_LIFECYCLE_RETRY_ATTEMPT_NOT_ADVANCED";

/// rule set is not owned by the requested pack
pub const CONFIGURATION_RULE_PACK_OWNERSHIP_MISMATCH: &str =
    "CONFIGURATION_RULE_PACK_OWNERSHIP_MISMATCH";

/// scope and key_name are required to delete a setting override
pub const CONFIGURATION_SETTING_DELETE_IDENTITY_REQUIRED: &str =
    "CONFIGURATION_SETTING_DELETE_IDENTITY_REQUIRED";

/// scope and key_name are required to read a setting
pub const CONFIGURATION_SETTING_READ_IDENTITY_REQUIRED: &str =
    "CONFIGURATION_SETTING_READ_IDENTITY_REQUIRED";

/// scope and key_name are required to update a setting
pub const CONFIGURATION_SETTING_UPDATE_IDENTITY_REQUIRED: &str =
    "CONFIGURATION_SETTING_UPDATE_IDENTITY_REQUIRED";

/// scope is required to list quality profiles
pub const CONFIGURATION_QUALITY_PROFILE_SCOPE_REQUIRED: &str =
    "CONFIGURATION_QUALITY_PROFILE_SCOPE_REQUIRED";

/// scope is required to replace quality profiles
pub const CONFIGURATION_QUALITY_PROFILE_REPLACEMENT_SCOPE_REQUIRED: &str =
    "CONFIGURATION_QUALITY_PROFILE_REPLACEMENT_SCOPE_REQUIRED";

/// setting value cannot be empty
pub const CONFIGURATION_SETTING_VALUE_EMPTY: &str = "CONFIGURATION_SETTING_VALUE_EMPTY";

/// subtitle provider config {id}
pub const CONFIGURATION_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND: &str =
    "CONFIGURATION_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND";

/// successful lifecycle sequence marker lacks complete step evidence
pub const CONFIGURATION_LIFECYCLE_SUCCESS_EVIDENCE_INCOMPLETE: &str =
    "CONFIGURATION_LIFECYCLE_SUCCESS_EVIDENCE_INCOMPLETE";

/// unknown setting key: {}.{}
pub const CONFIGURATION_UNKNOWN_SETTING_KEY: &str = "CONFIGURATION_UNKNOWN_SETTING_KEY";

#[cfg(test)]
const ALL: &[&str] = &[
    CONFIGURATION_LIFECYCLE_JOB_RUN_ID_REQUIRED,
    CONFIGURATION_SETTING_KEY_AMBIGUOUS,
    CONFIGURATION_CANDIDATE_EXPECTED_STATES_REQUIRED,
    CONFIGURATION_RULE_COPY_SOURCE_NOT_USER_RULE,
    CONFIGURATION_LIFECYCLE_STEP_PREFIX_MISMATCH,
    CONFIGURATION_LIFECYCLE_FAILED_STEP_ID_REQUIRED,
    CONFIGURATION_LIFECYCLE_DISPATCH_STATE_INVALID,
    CONFIGURATION_SUBTITLE_PROVIDER_UPDATE_EMPTY,
    CONFIGURATION_LIFECYCLE_STEP_TARGET_INVALID,
    CONFIGURATION_LIFECYCLE_STEP_ATTEMPT_INVALID,
    CONFIGURATION_LIFECYCLE_ATTEMPT_RESULT_REQUIRED,
    CONFIGURATION_LIFECYCLE_SEQUENCE_IDENTITY_MISMATCH,
    CONFIGURATION_LIFECYCLE_RECEIPT_EXPECTED_STATES_REQUIRED,
    CONFIGURATION_LIFECYCLE_CANDIDATE_ALREADY_ACTIVE,
    CONFIGURATION_LIFECYCLE_RULE_HAS_HISTORY,
    CONFIGURATION_LIFECYCLE_MEMBERSHIP_TERMINAL,
    CONFIGURATION_LIFECYCLE_CANDIDATE_PAGE_TOO_LARGE,
    CONFIGURATION_LIFECYCLE_RECEIPT_PAGE_TOO_LARGE,
    CONFIGURATION_LIFECYCLE_TERMINAL_STATE_MISMATCH,
    CONFIGURATION_LIFECYCLE_TERMINAL_STEP_IDS_INVALID,
    CONFIGURATION_LIFECYCLE_TERMINAL_EVIDENCE_MISMATCH,
    CONFIGURATION_PLUGIN_NOT_INSTALLED,
    CONFIGURATION_PLUGIN_SETTINGS_PLUGIN_REQUIRED,
    CONFIGURATION_LIFECYCLE_RETRY_ATTEMPT_NOT_ADVANCED,
    CONFIGURATION_RULE_PACK_OWNERSHIP_MISMATCH,
    CONFIGURATION_SETTING_DELETE_IDENTITY_REQUIRED,
    CONFIGURATION_SETTING_READ_IDENTITY_REQUIRED,
    CONFIGURATION_SETTING_UPDATE_IDENTITY_REQUIRED,
    CONFIGURATION_QUALITY_PROFILE_SCOPE_REQUIRED,
    CONFIGURATION_QUALITY_PROFILE_REPLACEMENT_SCOPE_REQUIRED,
    CONFIGURATION_SETTING_VALUE_EMPTY,
    CONFIGURATION_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND,
    CONFIGURATION_LIFECYCLE_SUCCESS_EVIDENCE_INCOMPLETE,
    CONFIGURATION_UNKNOWN_SETTING_KEY,
];

#[cfg(test)]
mod tests {
    #[test]
    fn reason_codes_are_distinct_and_stable_format() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(seen.insert(*code), "duplicate reason: {code}");
            assert!(
                ["CONFIGURATION_"]
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
