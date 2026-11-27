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
/// Retired: lifecycle rules defined `match` before their source declared
/// actions. No longer reported; kept so the code is never reused.
pub const RULE_MATCH_REQUIRED: &str = "RULE_MATCH_REQUIRED";
/// A lifecycle rule defines none of the action heads, so it could never do
/// anything to a subject.
pub const RULE_ACTION_REQUIRED: &str = "RULE_ACTION_REQUIRED";
/// A request matcher must define a vote, a shaping head, or `add_tags`.
pub const RULE_VOTE_REQUIRED: &str = "RULE_VOTE_REQUIRED";
/// A request rule defines both a head that decides a media request and a
/// head that decides a bad-file report.
pub const RULE_MIXES_REPORT_AND_REQUEST_HEADS: &str = "RULE_MIXES_REPORT_AND_REQUEST_HEADS";
/// A lifecycle action head that carries a value (`set_quality_profile`,
/// `add_tags`, `remove_tags`) is given something other than a plain literal.
pub const RULE_ACTION_VALUE_NOT_LITERAL: &str = "RULE_ACTION_VALUE_NOT_LITERAL";
/// A lifecycle action head is given a literal the action does not accept,
/// such as an empty profile name or a tag with an unsupported character.
pub const RULE_ACTION_VALUE_INVALID: &str = "RULE_ACTION_VALUE_INVALID";
/// The rule defines a name the host already provides as a helper.
pub const RULE_HOST_HELPER_REDEFINED: &str = "RULE_HOST_HELPER_REDEFINED";
/// A warning: a rule name is not an action head, not part of the contract,
/// and never used elsewhere in the rule, which usually means a misspelling.
pub const RULE_UNREFERENCED_RULE: &str = "RULE_UNREFERENCED_RULE";
/// A request rule name the host reads (a vote, an action head, or a contract
/// name such as `match` or `reasons`) is defined as a function, which never
/// reaches the decision.
pub const RULE_HEAD_DEFINED_AS_FUNCTION: &str = "RULE_HEAD_DEFINED_AS_FUNCTION";
/// A lifecycle action head is written as a function. A function of no
/// arguments still defines the head when evaluated, so any function under an
/// action name is refused rather than half understood.
pub const RULE_ACTION_HEAD_FUNCTION: &str = "RULE_ACTION_HEAD_FUNCTION";
/// The rule reads `data` outside its own package: a bare `data`, a computed
/// path, another package, or an import of one. The decision would then depend
/// on facts the host does not track.
pub const RULE_FOREIGN_DATA_REFERENCE: &str = "RULE_FOREIGN_DATA_REFERENCE";
/// A lifecycle rule uses `with` to replace its input, its `data`, or a
/// value the host's helpers depend on, so the decision would rest on facts
/// the host never observed.
pub const RULE_WITH_TARGET_NOT_ALLOWED: &str = "RULE_WITH_TARGET_NOT_ALLOWED";
/// `set_quality_profile` names a quality profile that does not exist.
pub const RULE_QUALITY_PROFILE_UNKNOWN: &str = "RULE_QUALITY_PROFILE_UNKNOWN";
/// `set_quality_profile` names a quality profile name that more than one
/// profile carries, so the rule does not say which one it means.
pub const RULE_QUALITY_PROFILE_AMBIGUOUS: &str = "RULE_QUALITY_PROFILE_AMBIGUOUS";
/// `set_monitor` names a monitor type no library the rule applies to accepts.
pub const RULE_MONITOR_UNSUPPORTED: &str = "RULE_MONITOR_UNSUPPORTED";
/// An audio language head names a language Scryer does not know.
pub const RULE_AUDIO_LANGUAGE_UNKNOWN: &str = "RULE_AUDIO_LANGUAGE_UNKNOWN";
/// A lifecycle action head reads a level of the input it does not act on: a
/// title head reads `input.season` or `input.episode`, a season head reads
/// `input.episode`, or an episode head reads `input.season`. Those sections
/// are missing at the head's level, so a negated read would always hold.
pub const RULE_ACTION_READS_OTHER_LEVEL: &str = "RULE_ACTION_READS_OTHER_LEVEL";
/// All supported diagnostic codes.
pub const ALL: &[&str] = &[
    RULE_ACTION_READS_OTHER_LEVEL,
    RULE_HEAD_DEFINED_AS_FUNCTION,
    RULE_QUALITY_PROFILE_UNKNOWN,
    RULE_QUALITY_PROFILE_AMBIGUOUS,
    RULE_MONITOR_UNSUPPORTED,
    RULE_AUDIO_LANGUAGE_UNKNOWN,
    RULE_WITH_TARGET_NOT_ALLOWED,
    RULE_ACTION_VALUE_NOT_LITERAL,
    RULE_ACTION_VALUE_INVALID,
    RULE_HOST_HELPER_REDEFINED,
    RULE_UNREFERENCED_RULE,
    RULE_ACTION_HEAD_FUNCTION,
    RULE_FOREIGN_DATA_REFERENCE,
    RULE_PACKAGE_REQUIRED,
    RULE_INPUT_IMPORT_UNSUPPORTED,
    RULE_MATCH_REQUIRED,
    RULE_ACTION_REQUIRED,
    RULE_VOTE_REQUIRED,
    RULE_MIXES_REPORT_AND_REQUEST_HEADS,
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
