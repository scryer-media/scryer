//! The merge preview summary (FR-071).
//!
//! A merge carries two things onto the destination title — the source's media
//! file records and its history — and drops the rest with the source title.
//! FR-071 asks the preview to state that, so [`MergePreviewSummary`] is exactly
//! five facts: which title survives, which media-file roles change, how much
//! history moves, how many source records go with the retired title, and what
//! (if anything) stops the merge.
//!
//! It is data rather than prose so every consumer — the GraphQL preview,
//! Activity, the confirmation dialog — renders the same decision the engine
//! would perform.

use serde::{Deserialize, Serialize};

use crate::location::merge::map::MergeBlockedRecord;
use crate::location::merge::roles::MediaRoleChange;

/// The complete FR-071 preview summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePreviewSummary {
    pub source_title_id: String,
    pub destination_title_id: String,
    /// The surviving title's name, as the catalog spells it (FR-071).
    ///
    /// Carried so every consumer can say "merges into “X”" without holding a
    /// title lookup of its own. `None` only for a summary built before the Group
    /// 0 snapshot could name the destination — the id is always there.
    #[serde(default)]
    pub destination_title_name: Option<String>,
    pub source_library_id: Option<String>,
    pub destination_library_id: Option<String>,

    /// FR-064: media file records repointed onto the surviving title.
    pub media_files_repointed: i64,
    /// FR-070: every role change, none of them silent.
    pub role_changes: Vec<MediaRoleChange>,
    /// How many of those role changes demote an incoming primary.
    pub role_demotions: i64,
    /// FR-064: history rows (`history_events` + `domain_events`) carried onto
    /// the surviving title.
    pub history_rows_carried: i64,
    /// FR-064: everything else recorded against the source title, as one count.
    /// Those rows retire with it through the ordinary title-delete path.
    pub source_records_dropped: i64,
    /// FR-066: non-empty means the merge cannot run.
    pub blocked: Vec<MergeBlockedRecord>,
}

impl MergePreviewSummary {
    /// Whether FR-066 stops this merge.
    pub fn is_blocked(&self) -> bool {
        !self.blocked.is_empty()
    }

    /// One line per blocked record, for
    /// `location_operation_title_checkpoints.blocked_reason`.
    pub fn blocked_reason(&self) -> Option<String> {
        if self.blocked.is_empty() {
            return None;
        }
        Some(
            self.blocked
                .iter()
                .map(MergeBlockedRecord::summary_line)
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::merge::map::MergeBlockReason;

    #[test]
    fn a_blocked_summary_renders_one_reason_line_per_record() {
        let summary = MergePreviewSummary {
            blocked: vec![
                MergeBlockedRecord {
                    table: "episodes".to_string(),
                    reason: MergeBlockReason::UnmappedEpisode,
                    source_id: "e-1".to_string(),
                    detail: "no destination episode carries standard S01E01".to_string(),
                },
                MergeBlockedRecord {
                    table: "download_submissions".to_string(),
                    reason: MergeBlockReason::ActiveAcquisitionWork,
                    source_id: "sub-1".to_string(),
                    detail: "a download is in flight".to_string(),
                },
            ],
            ..MergePreviewSummary::default()
        };
        assert!(summary.is_blocked());
        let reason = summary.blocked_reason().expect("a blocked summary has one");
        assert!(reason.contains("episodes (unmapped_episode): e-1"));
        assert!(reason.contains("download_submissions (active_acquisition_work): sub-1"));
    }

    #[test]
    fn a_clean_summary_has_no_reason() {
        assert!(MergePreviewSummary::default().blocked_reason().is_none());
    }
}
