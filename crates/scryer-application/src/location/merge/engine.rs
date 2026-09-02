//! The merge engine's seam: [`plan_merge`] builds the whole decision before
//! anything is written, [`execute_merge`] hands it to a repository that runs it
//! in **one** transaction.
//!
//! # The rule (FR-063–FR-067)
//!
//! The destination title wins everything except two things:
//!
//! 1. **Media file records.** The source's `media_files` rows are repointed at
//!    the destination title, with their episode and series-movie-link rows
//!    remapped and their roles resolved ([`super::roles`]). File-keyed
//!    dependents travel with the file, because `media_files.id` never changes.
//! 2. **History.** `history_events` and `domain_events` rows are unioned onto
//!    the destination, with episode ids remapped inside the payloads that carry
//!    them.
//!
//! Everything else recorded against the source title retires with it, through
//! the ordinary title-delete path. There is no per-table disposition list and no
//! foreign-key gate: the delete path already owns what a retired title leaves
//! behind.
//!
//! # How the executor wires this in
//!
//! At a title checkpoint whose classification is a merge, the executor:
//!
//! 1. calls [`TitleMergeRepository::load_merge_snapshot`] — read-only, outside
//!    any write transaction;
//! 2. calls [`plan_merge`], which either produces a [`MergePlan`] or fills its
//!    [`MergePreviewSummary::blocked`] set. A blocked plan never reaches step 3:
//!    the checkpoint goes to `TitleCheckpointState::Blocked` with
//!    [`MergePreviewSummary::blocked_reason`] as its `blocked_reason` (FR-066);
//! 3. calls [`execute_merge`], which repoints, unions, and deletes the source
//!    title row in one transaction and returns a [`MergeOutcome`];
//! 4. writes `merged_into_title_id` on the checkpoint and retires the source
//!    title's non-cascading dependents through the delete path.
//!
//! The same [`plan_merge`] call, with no execution, is the FR-071 preview. That
//! is deliberate: the preview and the execution are the same decision, so a
//! preview cannot describe a merge the engine would not perform.

use async_trait::async_trait;

use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::location::merge::map::{
    CollectionIdentityFacts, EpisodeIdentityFacts, MergeBlockedRecord, MergeIdentityInputs,
    MergeIdentityMap, MergeIdentityOutcome, SeriesMovieLinkIdentityFacts, evaluate_identity_map,
};
use crate::location::merge::roles::{
    FileEpisodeRoleRow, MergedRolePlan, TitleSlotFileRow, resolve_media_roles,
};
use crate::location::merge::summary::MergePreviewSummary;

/// `domain_events` types whose `payload_json` carries `$.data.episode_ids[]`.
/// Only these are decompressed, remapped, and recompressed; every other event
/// gets the cheap column-only rewrite.
///
/// `TitleContextSnapshot`, embedded in nearly every payload, carries no title
/// id — only `title_name`, `facet`, `external_ids`, `poster_url`, `year` — and
/// is never touched.
pub const EPISODE_BEARING_EVENT_TYPES: &[&str] = &[
    "release_grabbed",
    "download_failed",
    "release_blocklisted",
    "import_completed",
    "import_rejected",
    "media_file_analyzed",
    "media_file_renamed",
    "media_file_deleted",
    "media_file_upgraded",
];

/// Everything the read phase collects. Assembled by the repository; consumed by
/// [`plan_merge`], which performs no IO.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeCatalogSnapshot {
    pub source_title_id: String,
    pub destination_title_id: String,
    /// The surviving title's name, read with the rest of its row so the FR-071
    /// summary can name it instead of printing its id.
    pub destination_title_name: Option<String>,
    pub source_library_id: Option<String>,
    pub destination_library_id: Option<String>,

    pub source_episodes: Vec<EpisodeIdentityFacts>,
    pub destination_episodes: Vec<EpisodeIdentityFacts>,
    pub source_collections: Vec<CollectionIdentityFacts>,
    pub destination_collections: Vec<CollectionIdentityFacts>,
    pub source_links: Vec<SeriesMovieLinkIdentityFacts>,
    pub destination_links: Vec<SeriesMovieLinkIdentityFacts>,
    pub source_file_episode_rows: Vec<FileEpisodeRoleRow>,
    pub destination_file_episode_rows: Vec<FileEpisodeRoleRow>,
    /// Source media files that hang off the title rather than off an episode —
    /// a movie's file, or a title-level extra.
    pub source_title_slot_files: Vec<TitleSlotFileRow>,
    /// Whether the destination title already has a primary file for that slot.
    pub destination_title_slot_has_primary: bool,

    /// Source series-movie links a source media file is attached to.
    pub source_file_link_ids: std::collections::BTreeSet<String>,
    /// Source episode ids named by a history payload the merge carries.
    pub history_episode_ids: std::collections::BTreeSet<String>,
    /// Source collection ids named by a history payload the merge carries.
    pub history_collection_ids: std::collections::BTreeSet<String>,

    /// `media_files` rows the merge repoints.
    pub media_file_count: i64,
    /// `history_events` + `domain_events` rows the merge carries.
    pub history_row_count: i64,
    /// Everything else recorded against the source title, as one count.
    pub dropped_record_count: i64,

    /// Resumable location operations holding the source title.
    pub resumable_operations_holding_source: Vec<String>,
    /// Unconsumed manual-import selections on the source title.
    pub unconsumed_manual_import_selections: Vec<String>,
    /// Queued or in-flight download submissions on the source title.
    pub active_acquisition_work: Vec<String>,
}

impl MergeCatalogSnapshot {
    fn identity_inputs(&self) -> MergeIdentityInputs {
        let mut load_bearing_episode_ids = self.history_episode_ids.clone();
        load_bearing_episode_ids.extend(
            self.source_file_episode_rows
                .iter()
                .map(|row| row.episode_id.clone()),
        );
        MergeIdentityInputs {
            source_title_id: self.source_title_id.clone(),
            destination_title_id: self.destination_title_id.clone(),
            source_episodes: self.source_episodes.clone(),
            destination_episodes: self.destination_episodes.clone(),
            source_collections: self.source_collections.clone(),
            destination_collections: self.destination_collections.clone(),
            source_links: self.source_links.clone(),
            destination_links: self.destination_links.clone(),
            load_bearing_episode_ids,
            load_bearing_collection_ids: self.history_collection_ids.clone(),
            load_bearing_series_movie_link_ids: self.source_file_link_ids.clone(),
            resumable_operations_holding_source: self.resumable_operations_holding_source.clone(),
            unconsumed_manual_import_selections: self.unconsumed_manual_import_selections.clone(),
            active_acquisition_work: self.active_acquisition_work.clone(),
        }
    }
}

/// The decision, complete, before anything is written.
///
/// A plan with a non-empty [`MergePreviewSummary::blocked`] set is a *preview
/// of a refusal*: it is returned so the checkpoint can explain itself, and
/// [`execute_merge`] refuses to run it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePlan {
    pub source_title_id: String,
    pub destination_title_id: String,
    pub source_library_id: Option<String>,
    pub destination_library_id: Option<String>,
    /// `None` when the merge is blocked.
    pub identity_map: Option<MergeIdentityMap>,
    pub role_plan: MergedRolePlan,
    /// The source rows the role plan replaces, so the repoint can delete exactly
    /// what it is about to re-insert without re-deriving it.
    pub source_file_episode_rows: Vec<FileEpisodeRoleRow>,
    pub summary: MergePreviewSummary,
}

impl MergePlan {
    pub fn is_blocked(&self) -> bool {
        self.summary.is_blocked()
    }

    pub fn blocked(&self) -> &[MergeBlockedRecord] {
        &self.summary.blocked
    }

    /// The map, or an error naming the block. The executor calls this rather
    /// than unwrapping.
    pub fn require_identity_map(&self) -> AppResult<&MergeIdentityMap> {
        self.identity_map.as_ref().ok_or_else(|| {
            crate::AppError::Validation(
                self.summary
                    .blocked_reason()
                    .unwrap_or_else(|| "the merge has no identity map".to_string()),
            )
        })
    }
}

/// What the transaction did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeOutcome {
    pub source_title_id: String,
    pub destination_title_id: String,
    /// Rows affected per statement, for Activity and the operation counters.
    /// Keyed `"<step>:<table>"`, e.g. `"files:media_files"`.
    pub rows_affected: std::collections::BTreeMap<String, u64>,
    /// `domain_events` rows whose compressed payload was decompressed,
    /// remapped, and recompressed.
    pub domain_event_payloads_rewritten: u64,
}

/// The merge transaction, and the read that precedes it.
///
/// Declared here, in the application layer, following the local-trait pattern
/// the rest of `location` uses (see `executor::TitleFileMover`). The
/// implementation lives in `scryer-infrastructure-library`.
#[async_trait]
pub trait TitleMergeRepository: Send + Sync {
    /// The read phase. Read-only and outside any write transaction, so a blocked
    /// title costs no rollback.
    ///
    /// `current_operation_id` is the operation performing this merge. It is
    /// excluded from the resumable-operation check, because that operation
    /// legitimately owns the source title — the check is about a *second*
    /// operation still holding it.
    async fn load_merge_snapshot(
        &self,
        source_title_id: &str,
        destination_title_id: &str,
        current_operation_id: Option<&str>,
    ) -> AppResult<MergeCatalogSnapshot>;

    /// Repoint the media file records, union the history, delete the source
    /// title row — in one transaction, in that order, so no cascade can take a
    /// row the merge has already moved.
    async fn execute_title_merge(&self, plan: &MergePlan) -> AppResult<MergeOutcome>;
}

/// Build the merge decision from the snapshot. Pure.
pub fn plan_merge(snapshot: &MergeCatalogSnapshot) -> MergePlan {
    let outcome = evaluate_identity_map(&snapshot.identity_inputs());

    let (identity_map, blocked) = match outcome {
        MergeIdentityOutcome::Mapped(map) => (Some(*map), Vec::new()),
        MergeIdentityOutcome::Blocked(records) => (None, records),
    };

    let role_plan = identity_map
        .as_ref()
        .map(|map| {
            resolve_media_roles(
                map,
                &snapshot.source_file_episode_rows,
                &snapshot.destination_file_episode_rows,
                &snapshot.source_title_slot_files,
                snapshot.destination_title_slot_has_primary,
            )
        })
        .unwrap_or_default();

    let summary = MergePreviewSummary {
        source_title_id: snapshot.source_title_id.clone(),
        destination_title_id: snapshot.destination_title_id.clone(),
        destination_title_name: snapshot.destination_title_name.clone(),
        source_library_id: snapshot.source_library_id.clone(),
        destination_library_id: snapshot.destination_library_id.clone(),
        media_files_repointed: snapshot.media_file_count,
        role_changes: role_plan.role_changes.clone(),
        role_demotions: role_plan.demotion_count() as i64,
        history_rows_carried: snapshot.history_row_count,
        source_records_dropped: snapshot.dropped_record_count,
        blocked,
    };

    MergePlan {
        source_title_id: snapshot.source_title_id.clone(),
        destination_title_id: snapshot.destination_title_id.clone(),
        source_library_id: snapshot.source_library_id.clone(),
        destination_library_id: snapshot.destination_library_id.clone(),
        identity_map,
        role_plan,
        source_file_episode_rows: snapshot.source_file_episode_rows.clone(),
        summary,
    }
}

/// Run a planned merge. Refuses a blocked plan without touching the database —
/// FR-066's block is decided in the read phase and must never cost a rollback.
pub async fn execute_merge(
    repository: &dyn TitleMergeRepository,
    plan: &MergePlan,
) -> AppResult<MergeOutcome> {
    if plan.is_blocked() {
        return Err(crate::AppError::Validation(format!(
            "merge of title {} into {} is blocked: {}",
            plan.source_title_id,
            plan.destination_title_id,
            plan.summary
                .blocked_reason()
                .unwrap_or_else(|| "unmappable records".to_string())
        )));
    }
    plan.require_identity_map()?;
    repository.execute_title_merge(plan).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::merge::MergedMediaRole;
    use crate::location::merge::map::MergeBlockReason;
    use scryer_domain::EpisodeType;
    use std::collections::BTreeSet;

    fn episode(id: &str, season: &str, number: &str) -> EpisodeIdentityFacts {
        EpisodeIdentityFacts {
            id: id.to_string(),
            episode_type: EpisodeType::Standard,
            season_number: Some(season.to_string()),
            episode_number: Some(number.to_string()),
            absolute_number: None,
            collection_id: None,
        }
    }

    fn snapshot() -> MergeCatalogSnapshot {
        MergeCatalogSnapshot {
            source_title_id: "source".to_string(),
            destination_title_id: "destination".to_string(),
            destination_title_name: Some("The Surviving Title".to_string()),
            source_library_id: Some("library-a".to_string()),
            destination_library_id: Some("library-b".to_string()),
            source_episodes: vec![episode("s-e1", "1", "1")],
            destination_episodes: vec![episode("d-e1", "1", "1")],
            source_file_episode_rows: vec![FileEpisodeRoleRow {
                file_id: "file-in".to_string(),
                episode_id: "s-e1".to_string(),
                role: MergedMediaRole::Primary,
                is_filler: false,
            }],
            destination_file_episode_rows: vec![FileEpisodeRoleRow {
                file_id: "file-dest".to_string(),
                episode_id: "d-e1".to_string(),
                role: MergedMediaRole::Primary,
                is_filler: false,
            }],
            media_file_count: 3,
            history_row_count: 12,
            dropped_record_count: 7,
            ..MergeCatalogSnapshot::default()
        }
    }

    #[test]
    fn a_clean_snapshot_plans_a_complete_merge() {
        let plan = plan_merge(&snapshot());
        assert!(!plan.is_blocked());
        let map = plan.require_identity_map().expect("the map exists");
        assert_eq!(map.episode("s-e1"), Some("d-e1"));
        // FR-071: the summary names the surviving title, so no consumer has to
        // resolve the destination id back into something a user can read.
        assert_eq!(
            plan.summary.destination_title_name.as_deref(),
            Some("The Surviving Title")
        );
        // FR-070: the demotion is in the plan and in the summary.
        assert_eq!(plan.role_plan.demotion_count(), 1);
        assert_eq!(plan.summary.role_changes.len(), 1);
        assert_eq!(plan.summary.role_demotions, 1);
        // FR-064: the three counts the preview reports.
        assert_eq!(plan.summary.media_files_repointed, 3);
        assert_eq!(plan.summary.history_rows_carried, 12);
        assert_eq!(plan.summary.source_records_dropped, 7);
    }

    #[test]
    fn an_unmapped_episode_carrying_a_file_blocks_the_plan_and_the_execution() {
        let mut snapshot = snapshot();
        snapshot.source_episodes.push(episode("s-e2", "1", "2"));
        snapshot.source_file_episode_rows.push(FileEpisodeRoleRow {
            file_id: "file-two".to_string(),
            episode_id: "s-e2".to_string(),
            role: MergedMediaRole::Primary,
            is_filler: false,
        });
        let plan = plan_merge(&snapshot);
        assert!(plan.is_blocked());
        assert!(plan.identity_map.is_none());
        assert!(
            plan.blocked()
                .iter()
                .any(|record| record.table == "episodes"
                    && record.reason == MergeBlockReason::UnmappedEpisode)
        );
        assert!(plan.require_identity_map().is_err());
        // No role plan is computed for a blocked merge.
        assert!(plan.role_plan.rows.is_empty());
    }

    #[test]
    fn an_unmapped_episode_carrying_nothing_does_not_block() {
        let mut snapshot = snapshot();
        snapshot.source_episodes.push(episode("s-e2", "1", "2"));
        let plan = plan_merge(&snapshot);
        assert!(!plan.is_blocked());
        assert!(
            plan.require_identity_map()
                .expect("the map exists")
                .episode("s-e2")
                .is_none()
        );
    }

    #[test]
    fn a_history_only_episode_reference_still_blocks() {
        let mut snapshot = snapshot();
        snapshot.source_episodes.push(episode("s-e2", "1", "2"));
        snapshot.history_episode_ids = BTreeSet::from(["s-e2".to_string()]);
        assert!(plan_merge(&snapshot).is_blocked());
    }

    #[test]
    fn a_live_download_on_the_source_blocks_the_merge() {
        let mut snapshot = snapshot();
        snapshot.active_acquisition_work = vec!["submission-1".to_string()];
        let plan = plan_merge(&snapshot);
        assert!(plan.is_blocked());
        assert!(
            plan.summary
                .blocked_reason()
                .expect("a reason")
                .contains("active_acquisition_work")
        );
    }
}
