//! Stable names for the conditions an episode-file reassignment is refused on.
//!
//! Each one travels beside the English sentence as `extensions.reason` on a
//! `VALIDATION_ERROR`, so a client can say the same thing in its own language
//! instead of showing or matching the sentence. A name states the condition,
//! never the wording, and is API once released: reword a sentence freely, but
//! do not rename or reuse its reason.
//!
//! The values a client needs to act on a refusal travel as their own
//! extensions, named by the `*_FACT` keys below.

/// No target episode was given.
pub const NO_EPISODES: &str = "EPISODE_FILE_REASSIGN_NO_EPISODES";
/// A target episode does not belong to the file's title.
pub const EPISODE_OUTSIDE_TITLE: &str = "EPISODE_FILE_REASSIGN_EPISODE_OUTSIDE_TITLE";
/// A target episode already has a primary file and replacing it was not asked
/// for. Carries [`CONFLICTING_FILE_ID_FACT`] and [`EPISODE_ID_FACT`].
pub const PRIMARY_CONFLICT: &str = "EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT";
/// The file's title is not a series or anime, so it has no episodes.
pub const NOT_EPISODIC_TITLE: &str = "EPISODE_FILE_REASSIGN_NOT_EPISODIC_TITLE";
/// The file is not a single episode file that can move between episodes: it
/// serves a series movie, or it is a disc image.
pub const UNSUPPORTED_FILE: &str = "EPISODE_FILE_REASSIGN_UNSUPPORTED_FILE";
/// The name the new episodes render is already taken by another file on disk.
pub const DESTINATION_OCCUPIED: &str = "EPISODE_FILE_REASSIGN_DESTINATION_OCCUPIED";
/// The renamer could not plan a name for the new episodes (the file is outside
/// its library roots, the naming template does not render, and the like).
/// Carries [`RENAME_REASON_FACT`].
pub const RENAME_REFUSED: &str = "EPISODE_FILE_REASSIGN_RENAME_REFUSED";
/// Moving the file to its new name failed. The file and its episodes were left
/// as they were.
pub const RENAME_FAILED: &str = "EPISODE_FILE_REASSIGN_RENAME_FAILED";

pub const ALL: &[&str] = &[
    NO_EPISODES,
    EPISODE_OUTSIDE_TITLE,
    PRIMARY_CONFLICT,
    NOT_EPISODIC_TITLE,
    UNSUPPORTED_FILE,
    DESTINATION_OCCUPIED,
    RENAME_REFUSED,
    RENAME_FAILED,
];

/// The id of the file that is primary for a contested target episode.
pub const CONFLICTING_FILE_ID_FACT: &str = "conflictingFileId";
/// The target episode a refusal is about.
pub const EPISODE_ID_FACT: &str = "episodeId";
/// The renamer's own reason code for a plan it would not make.
pub const RENAME_REASON_FACT: &str = "renameReason";

pub const ALL_FACTS: &[&str] = &[
    CONFLICTING_FILE_ID_FACT,
    EPISODE_ID_FACT,
    RENAME_REASON_FACT,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppError, RefusalFact};
    use std::collections::BTreeSet;

    #[test]
    fn every_reason_is_a_distinct_episode_file_reassign_name_in_screaming_snake() {
        let distinct = ALL.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), ALL.len(), "a reason is listed twice");
        for reason in ALL {
            assert!(reason.starts_with("EPISODE_FILE_REASSIGN_"), "{reason}");
            assert!(
                reason
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{reason}"
            );
            assert!(!reason.ends_with('_') && !reason.contains("__"), "{reason}");
        }
    }

    #[test]
    fn every_fact_key_is_a_distinct_camel_case_extension_name() {
        let distinct = ALL_FACTS.iter().collect::<BTreeSet<_>>();
        assert_eq!(
            distinct.len(),
            ALL_FACTS.len(),
            "a fact key is listed twice"
        );
        for key in ALL_FACTS {
            assert!(key.chars().all(|c| c.is_ascii_alphanumeric()), "{key}");
            assert!(
                key.chars().next().is_some_and(|c| c.is_ascii_lowercase()),
                "{key}"
            );
            assert!(!matches!(*key, "code" | "reason"), "{key}");
        }
    }

    #[test]
    fn a_conflict_refusal_carries_its_facts_beside_the_reason() {
        let error = AppError::validation_refused_with_facts(
            PRIMARY_CONFLICT,
            "episode already has a primary file",
            vec![
                RefusalFact::new(CONFLICTING_FILE_ID_FACT, "file-b"),
                RefusalFact::new(EPISODE_ID_FACT, "episode-2"),
            ],
        );
        assert_eq!(error.validation_reason(), Some(PRIMARY_CONFLICT));
        assert_eq!(
            error.to_string(),
            "validation: episode already has a primary file"
        );
        assert_eq!(
            error.refusal_facts(),
            &[
                RefusalFact::new(CONFLICTING_FILE_ID_FACT, "file-b"),
                RefusalFact::new(EPISODE_ID_FACT, "episode-2"),
            ]
        );
        assert!(
            AppError::validation_refused(NO_EPISODES, "x")
                .refusal_facts()
                .is_empty()
        );
    }
}
