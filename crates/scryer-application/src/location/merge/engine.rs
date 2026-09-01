//! The merge engine's seam: [`plan_merge`] builds the whole decision before
//! anything is written, [`execute_merge`] hands it to a repository that runs
//! `merge-inventory.md` §8's Groups 1–5 in **one** transaction.
//!
//! # How the executor wires this in (T085 → the title-checkpoint package)
//!
//! At a title checkpoint whose classification is a merge, the executor:
//!
//! 1. calls [`TitleMergeRepository::load_merge_snapshot`] — Group 0, read-only,
//!    outside any write transaction;
//! 2. calls [`plan_merge`], which either produces a [`MergePlan`] or fills its
//!    [`MergePreviewSummary::blocked`] set. A blocked plan never reaches step 3:
//!    the checkpoint goes to `TitleCheckpointState::Blocked` with
//!    [`MergePreviewSummary::blocked_reason`] as its `blocked_reason` (FR-066);
//! 3. calls [`execute_merge`], which runs Groups 1–5 transactionally and
//!    returns a [`MergeOutcome`];
//! 4. writes `merged_into_title_id` on the checkpoint and schedules
//!    [`MergeOutcome::post_merge_work`] (Group 6) — derived-cache rebuilds that
//!    are idempotent and safe to lose to a crash.
//!
//! The same [`plan_merge`] call, with no execution, is the FR-071 preview. That
//! is deliberate: the preview and the execution are the same decision, so a
//! preview cannot describe a merge the engine would not perform.
//!
//! # Live-schema deviations from the T081 inventory
//!
//! `merge-inventory.md`'s schema reconstruction replayed `CREATE TABLE` and
//! `ALTER TABLE` but not `DROP TABLE`, so five of its tables do not exist in
//! the live schema. They are recorded in [`INVENTORY_DEVIATIONS`] and the
//! engine touches none of them.

use async_trait::async_trait;

use serde::{Deserialize, Serialize};

use crate::AppResult;
use crate::location::merge::MergeDisposition;
use crate::location::merge::map::{
    CollectionIdentityFacts, EpisodeIdentityFacts, MergeBlockedRecord, MergeIdentityInputs,
    MergeIdentityMap, MergeIdentityOutcome, SeriesMovieLinkIdentityFacts, evaluate_identity_map,
};
use crate::location::merge::roles::{FileEpisodeRoleRow, MergedRolePlan, resolve_media_roles};
use crate::location::merge::summary::{
    DestinationWinsEntry, DroppedCategory, MediaRequestRepoint, MergePreviewSummary, PostMergeWork,
    TableDispositionEntry, TagMergeResult, partition_tags,
};

/// Tables with **no foreign key** to `titles` or `episodes`. Nothing in the
/// database removes their source-id rows when the source title row goes, so the
/// FR-067 gate asserts on them explicitly before the delete — a missed rewrite
/// here is otherwise completely invisible (`merge-inventory.md` §8 Group 5).
///
/// Each entry is `(table, column, kind)` where `kind` says whether the column
/// holds a source *title* id or a source *episode* id.
pub const FR067_NO_FK_ASSERTIONS: &[(&str, &str, MergeGateIdKind)] = &[
    ("domain_events", "title_id", MergeGateIdKind::Title),
    ("domain_events", "stream_id", MergeGateIdKind::TitleStream),
    ("download_submissions", "title_id", MergeGateIdKind::Title),
    ("download_submissions", "episode_id", MergeGateIdKind::Episode),
    (
        "download_submission_episode_links",
        "episode_id",
        MergeGateIdKind::Episode,
    ),
    ("subtitle_downloads", "episode_id", MergeGateIdKind::Episode),
    (
        "post_processing_script_runs",
        "title_id",
        MergeGateIdKind::Title,
    ),
    ("manual_import_selections", "title_id", MergeGateIdKind::Title),
    ("indexer_search_learning", "title_id", MergeGateIdKind::Title),
    (
        "media_server_playback_items",
        "entity_id",
        MergeGateIdKind::PlaybackEntity,
    ),
    (
        "discovery_item_library_provenance",
        "title_id",
        MergeGateIdKind::Title,
    ),
    (
        "discovery_pending_context_changes",
        "previous_title_id",
        MergeGateIdKind::Title,
    ),
    ("pending_releases", "title_id", MergeGateIdKind::Title),
    ("release_decisions", "title_id", MergeGateIdKind::Title),
    (
        "location_operation_verifications",
        "title_id",
        MergeGateIdKind::Title,
    ),
];

/// Tables whose foreign key is `ON DELETE SET NULL`. A missed rewrite here does
/// not dangle — it produces a surviving row with no title, which reads as an
/// orphan in Activity and in the API. Quieter than the no-FK class, so it is
/// the gate's second-priority assertion (`merge-inventory.md` §8 Group 5).
pub const FR067_SET_NULL_ASSERTIONS: &[(&str, &str, MergeGateIdKind)] = &[
    ("history_events", "title_id", MergeGateIdKind::Title),
    ("workflow_operations", "title_id", MergeGateIdKind::Title),
    ("workflow_operations", "episode_id", MergeGateIdKind::Episode),
    (
        "download_import_artifacts",
        "title_id",
        MergeGateIdKind::Title,
    ),
    (
        "download_import_artifacts",
        "episode_id",
        MergeGateIdKind::Episode,
    ),
    ("media_requests", "created_title_id", MergeGateIdKind::Title),
    (
        "release_download_attempts",
        "title_id",
        MergeGateIdKind::Title,
    ),
    ("discovery_titles", "resolved_title_id", MergeGateIdKind::Title),
    (
        "discovery_submitted_subjects",
        "title_id",
        MergeGateIdKind::Title,
    ),
    (
        "discovery_pending_context_changes",
        "title_id",
        MergeGateIdKind::Title,
    ),
];

/// What kind of source id a gate column holds, because the assertion query
/// differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeGateIdKind {
    /// The column holds a source title id.
    Title,
    /// `domain_events.stream_id`, which is a title id only when
    /// `stream_kind = 'title'`.
    TitleStream,
    /// The column holds a source episode id.
    Episode,
    /// `media_server_playback_items.entity_id`, a title id when
    /// `entity_kind = 'title'` and an episode id when it is `'episode'`.
    PlaybackEntity,
}

/// `domain_events` types whose `payload_json` carries `$.data.episode_ids[]`
/// (`merge-inventory.md` §6 OQ8). Only these are decompressed, remapped, and
/// recompressed; every other event gets the cheap column-only rewrite.
///
/// `TitleContextSnapshot`, embedded in nearly every payload, carries no title
/// id — only `title_name`, `facet`, `external_ids`, `poster_url`, `year` — and
/// is never touched under any option.
pub const OQ8_EPISODE_BEARING_EVENT_TYPES: &[&str] = &[
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

/// Tables `merge-inventory.md` classifies but the live schema does not have,
/// with the migration that removed each. Surfaced in the preview's notes so a
/// reviewer comparing the engine against the appendix finds the answer here
/// rather than assuming an omission.
pub const INVENTORY_DEVIATIONS: &[(&str, &str)] = &[
    (
        "releases",
        "dropped by sqlite migration 0122 and absent from the postgres baseline; OQ1's `drop` is a \
         no-op",
    ),
    (
        "policy_decisions",
        "dropped by sqlite migration 0011 and absent from the postgres baseline; OQ2's `drop` is a \
         no-op",
    ),
    (
        "title_aliases",
        "dropped by sqlite migration 0122, never present on postgres, and referenced by no Rust \
         SQL; the §3 `union with dedupe` has nothing to union",
    ),
    (
        "title_history",
        "dropped by sqlite migration 0085; §2 already called it dead schema, and it is not merely \
         unread but absent",
    ),
    (
        "quarantine_items",
        "dropped by sqlite migration 0122; the §4 `no rewrite needed` entry has no table",
    ),
    (
        "subtitle_blacklist",
        "renamed to `subtitle_blocklist` by sqlite migration 0094; still media-file keyed, so \
         still needs no rewrite",
    ),
    (
        "wanted_items",
        "§2 justifies destination-wins by `next_search_at`, `search_count`, and `search_phase`, \
         all three dropped by migration 0143. The disposition stands — `status` and the \
         acquisition cursor still live on the destination row — but the stated reason is stale",
    ),
    (
        "title_external_ids",
        "§3 names three unique indexes; migrations 0079/0104/0105 left only \
         `idx_title_external_ids_library_lookup`. The collision is still a certainty under \
         FR-055, so the disposition (delete the source rows, never repoint) is unchanged",
    ),
];

/// Everything Group 0 reads. Assembled by the repository; consumed by
/// [`plan_merge`], which performs no IO.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeCatalogSnapshot {
    pub source_title_id: String,
    pub destination_title_id: String,
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

    pub source_tags: Vec<String>,
    pub destination_tags: Vec<String>,

    /// Table → the source episode ids that table's rows reference, for the
    /// FR-066 evaluation.
    pub episode_references: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    /// Table → source row count, for the preview's per-disposition counts.
    pub source_row_counts: std::collections::BTreeMap<String, i64>,
    /// `media_requests` rows on the source title, for the OQ10 repoint note.
    pub media_request_ids: Vec<String>,

    /// OQ7: resumable location operations holding the source title.
    pub resumable_operations_holding_source: Vec<String>,
    /// Unconsumed manual-import selections on the source title.
    pub unconsumed_manual_import_selections: Vec<String>,
}

impl MergeCatalogSnapshot {
    fn identity_inputs(&self) -> MergeIdentityInputs {
        MergeIdentityInputs {
            source_title_id: self.source_title_id.clone(),
            destination_title_id: self.destination_title_id.clone(),
            source_episodes: self.source_episodes.clone(),
            destination_episodes: self.destination_episodes.clone(),
            source_collections: self.source_collections.clone(),
            destination_collections: self.destination_collections.clone(),
            source_links: self.source_links.clone(),
            destination_links: self.destination_links.clone(),
            episode_references: self.episode_references.clone(),
            resumable_operations_holding_source: self.resumable_operations_holding_source.clone(),
            unconsumed_manual_import_selections: self.unconsumed_manual_import_selections.clone(),
        }
    }

    fn rows(&self, table: &str) -> i64 {
        self.source_row_counts.get(table).copied().unwrap_or(0)
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
    /// The source rows the role plan replaces, so Group 1 can delete exactly
    /// what it is about to re-insert without re-deriving it.
    pub source_file_episode_rows: Vec<FileEpisodeRoleRow>,
    pub tags: TagMergeResult,
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
    /// Rows affected per statement group, for Activity and the operation
    /// counters. Keyed `"<group>:<table>"`, e.g. `"1:media_files"`.
    pub rows_affected: std::collections::BTreeMap<String, u64>,
    /// `domain_events` rows whose compressed payload was decompressed,
    /// remapped, and recompressed (OQ8's middle path).
    pub domain_event_payloads_rewritten: u64,
    /// Group 6, for the caller to schedule. The engine never reaches into those
    /// subsystems itself.
    pub post_merge_work: Vec<PostMergeWork>,
}

/// The Groups 1–5 transaction, and the Group 0 read that precedes it.
///
/// Declared here, in the application layer, following the local-trait pattern
/// the rest of `location` uses (see `executor::TitleFileMover`). The
/// implementation lives in `scryer-infrastructure-library`.
#[async_trait]
pub trait TitleMergeRepository: Send + Sync {
    /// Group 0. Read-only and outside any write transaction, so a blocked title
    /// costs no rollback.
    ///
    /// `current_operation_id` is the operation performing this merge. It is
    /// excluded from the OQ7 resumable-operation check, because that operation
    /// legitimately owns the source title — OQ7 is about a *second* operation
    /// still holding it.
    async fn load_merge_snapshot(
        &self,
        source_title_id: &str,
        destination_title_id: &str,
        current_operation_id: Option<&str>,
    ) -> AppResult<MergeCatalogSnapshot>;

    /// Groups 1–5 in a single transaction, in the order
    /// `merge-inventory.md` §8 forces, ending at the FR-067 gate: the source
    /// `titles` row is deleted only after the no-FK and SET NULL assertion
    /// lists come back empty. A failed assertion aborts the transaction with an
    /// error naming the table.
    async fn execute_title_merge(&self, plan: &MergePlan) -> AppResult<MergeOutcome>;
}

/// Build the merge decision from the Group 0 snapshot. Pure.
pub fn plan_merge(snapshot: &MergeCatalogSnapshot) -> MergePlan {
    let outcome = evaluate_identity_map(&snapshot.identity_inputs());
    let tags = partition_tags(&snapshot.source_tags, &snapshot.destination_tags);

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
            )
        })
        .unwrap_or_default();

    let mut summary = MergePreviewSummary {
        source_title_id: snapshot.source_title_id.clone(),
        destination_title_id: snapshot.destination_title_id.clone(),
        source_library_id: snapshot.source_library_id.clone(),
        destination_library_id: snapshot.destination_library_id.clone(),
        destination_wins: destination_wins_entries(snapshot),
        dispositions: disposition_entries(snapshot),
        blocked,
        role_changes: role_plan.role_changes.clone(),
        reserved_tag_conflicts: tags.reserved_tag_conflicts.clone(),
        free_form_tags_added: tags.free_form_tags_added.clone(),
        media_request_repoints: media_request_repoints(snapshot),
        dropped: dropped_categories(snapshot),
        post_merge_work: vec![
            PostMergeWork::ReindexTitleSearchTerms,
            PostMergeWork::RegenerateRecommendations,
            PostMergeWork::RecomputeStatistics,
            PostMergeWork::DropSourceIndexerCoverage,
        ],
        notes: Vec::new(),
    };

    // OQ6: the merge writes nothing outside the database, so a recycle-bin
    // manifest recorded under the source title keeps a title id that no longer
    // resolves. Stated, never silently accepted.
    summary.notes.push(
        "Recycle-bin manifests recorded under the source title keep its title id; the merge \
         performs no filesystem writes (OQ6)."
            .to_string(),
    );
    // OQ8: what the payload rewrite does and does not cover.
    summary.notes.push(format!(
        "Activity events keep their title: `title_id` and title `stream_id` are rewritten for every \
         event, and `$.data.episode_ids[]` is remapped for the {} event types that carry it (OQ8).",
        OQ8_EPISODE_BEARING_EVENT_TYPES.len()
    ));
    if !tags.reserved_tags_dropped.is_empty() {
        summary.notes.push(format!(
            "{} reserved `scryer:` tag(s) matched the destination's value and were dropped without \
             a conflict (OQ9).",
            tags.reserved_tags_dropped.len()
        ));
    }
    if let Some(map) = identity_map.as_ref()
        && !map.unevaluated_reference_tables.is_empty()
    {
        summary.notes.push(format!(
            "Episode references were supplied for table(s) outside the FR-066 blocking set and \
             were not evaluated: {}.",
            map.unevaluated_reference_tables.join(", ")
        ));
    }
    if !role_plan.unmapped_rows.is_empty() {
        summary.notes.push(format!(
            "{} file_episode_map row(s) reference an episode outside the identity map.",
            role_plan.unmapped_rows.len()
        ));
    }
    for (table, reason) in INVENTORY_DEVIATIONS {
        summary
            .notes
            .push(format!("Inventory deviation — `{table}`: {reason}."));
    }

    MergePlan {
        source_title_id: snapshot.source_title_id.clone(),
        destination_title_id: snapshot.destination_title_id.clone(),
        source_library_id: snapshot.source_library_id.clone(),
        destination_library_id: snapshot.destination_library_id.clone(),
        identity_map,
        role_plan,
        source_file_episode_rows: snapshot.source_file_episode_rows.clone(),
        tags,
        summary,
    }
}

/// Run a planned merge. Refuses a blocked plan without touching the database —
/// FR-066's block is decided in Group 0 and must never cost a rollback.
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

fn destination_wins_entries(snapshot: &MergeCatalogSnapshot) -> Vec<DestinationWinsEntry> {
    // FR-063's list, in the order the spec states it.
    vec![
        DestinationWinsEntry {
            setting: "title id".to_string(),
            destination_value: Some(snapshot.destination_title_id.clone()),
            source_value: Some(snapshot.source_title_id.clone()),
        },
        DestinationWinsEntry {
            setting: "metadata identity".to_string(),
            destination_value: None,
            source_value: None,
        },
        DestinationWinsEntry {
            setting: "monitoring".to_string(),
            destination_value: None,
            source_value: None,
        },
        DestinationWinsEntry {
            setting: "explicit settings".to_string(),
            destination_value: None,
            source_value: None,
        },
        DestinationWinsEntry {
            setting: "quality configuration".to_string(),
            destination_value: None,
            source_value: None,
        },
        DestinationWinsEntry {
            setting: "naming behavior".to_string(),
            destination_value: None,
            source_value: None,
        },
        DestinationWinsEntry {
            setting: "library inheritance".to_string(),
            destination_value: snapshot.destination_library_id.clone(),
            source_value: snapshot.source_library_id.clone(),
        },
    ]
}

fn disposition_entries(snapshot: &MergeCatalogSnapshot) -> Vec<TableDispositionEntry> {
    let entry = |table: &str, disposition: MergeDisposition, note: &str| TableDispositionEntry {
        table: table.to_string(),
        disposition,
        source_row_count: snapshot.rows(table),
        note: note.to_string(),
    };
    let mut entries = vec![
        // Group 1.
        entry(
            "media_files",
            MergeDisposition::Union,
            "title_id is repointed; the file id and file_path are untouched",
        ),
        entry(
            "file_episode_map",
            MergeDisposition::Map,
            "episode ids remap and roles resolve per FR-068/069/070",
        ),
        entry(
            "file_series_movie_link_map",
            MergeDisposition::Map,
            "the link id changes, the file id does not",
        ),
        // Group 2.
        entry(
            "wanted_items",
            MergeDisposition::Union,
            "destination-wins on UNIQUE(title_id, episode_id) and the two partial indexes (OQ5)",
        ),
        entry(
            "download_submissions",
            MergeDisposition::Union,
            "client-keyed uniqueness, so a union cannot collide",
        ),
        entry(
            "download_submission_episode_links",
            MergeDisposition::Map,
            "two source episodes collapsing onto one destination episode resolve ON CONFLICT DO \
             NOTHING",
        ),
        entry(
            "download_import_artifacts",
            MergeDisposition::Union,
            "title, episode, and imported file references all remap",
        ),
        entry(
            "subtitle_downloads",
            MergeDisposition::Union,
            "title_id and episode_id remap; media_file_id is already stable",
        ),
        entry(
            "workflow_operations",
            MergeDisposition::Map,
            "an audit row should describe the surviving identity",
        ),
        entry(
            "domain_events",
            MergeDisposition::Union,
            "title_id and title stream_id rewritten for every event; episode_ids remapped for the \
             event types that carry them (OQ8)",
        ),
        entry(
            "media_server_playback_items",
            MergeDisposition::Map,
            "destination-wins on the (connection, kind, entity) primary key",
        ),
        // Group 3.
        entry(
            "blocklist",
            MergeDisposition::Union,
            "deduped ON CONFLICT DO NOTHING against both 0194 partial indexes; title-scoped since \
             0194, so not in the FR-066 blocking set",
        ),
        entry(
            "release_download_attempts",
            MergeDisposition::Union,
            "title_id repointed",
        ),
        entry(
            "post_processing_script_runs",
            MergeDisposition::Union,
            "title_id only; title_name and env_payload_json are deliberate historical snapshots",
        ),
        entry(
            "media_requests",
            MergeDisposition::Union,
            "created_title_id remaps and library_id is repointed to the destination (OQ10)",
        ),
        entry(
            "imports",
            MergeDisposition::Map,
            "a JSON rewrite of payload_json $.target_title_id / $.manual_title_id",
        ),
        entry(
            "indexer_search_learning",
            MergeDisposition::Union,
            "destination-wins on the (indexer, title, facet, strategy) primary key; counters are \
             never summed (OQ3)",
        ),
        entry(
            "discovery_titles",
            MergeDisposition::Map,
            "resolved_title_id remaps",
        ),
        entry(
            "discovery_item_library_provenance",
            MergeDisposition::Map,
            "title_id and library_id remap, deduped ON CONFLICT DO NOTHING",
        ),
        entry(
            "discovery_submitted_subjects",
            MergeDisposition::Map,
            "title_id and library_id remap",
        ),
        entry(
            "discovery_pending_context_changes",
            MergeDisposition::Map,
            "both title_id and previous_title_id remap",
        ),
        entry(
            "image_proxy_sources",
            MergeDisposition::Map,
            "owner_id remaps where owner_type denotes a title",
        ),
        entry(
            "location_operation_owned_entities",
            MergeDisposition::Map,
            "a live claim must follow the surviving title",
        ),
        entry(
            "location_operation_title_checkpoints",
            MergeDisposition::Map,
            "merged_into_title_id only; title_id is the operation's own audit trail",
        ),
        entry(
            "titles.tags",
            MergeDisposition::Union,
            "free-form tags union; reserved scryer: tags are destination-wins (OQ9)",
        ),
        // Group 4/5.
        entry(
            "title_external_ids",
            MergeDisposition::DestinationWins,
            "source rows are deleted, never repointed: FR-055 guarantees both sides share \
             (source, external_id)",
        ),
        entry(
            "episode_external_ids",
            MergeDisposition::DestinationWins,
            "FR-063 gives the destination the metadata identity",
        ),
        entry(
            "collection_external_ids",
            MergeDisposition::DestinationWins,
            "FR-065 gives duplicate collection metadata to the destination",
        ),
        entry(
            "title_images",
            MergeDisposition::DestinationWins,
            "UNIQUE(title_id, kind) makes a union structurally impossible",
        ),
        entry(
            "title_metadata_tags",
            MergeDisposition::DestinationWins,
            "hydrated from the destination's metadata identity",
        ),
        entry(
            "title_credits",
            MergeDisposition::DestinationWins,
            "hydrated from the destination's metadata identity",
        ),
        entry(
            "library_probe_signatures",
            MergeDisposition::DestinationWins,
            "the signature describes a folder path; the source's is stale after the move",
        ),
        entry(
            "title_search_terms",
            MergeDisposition::Drop,
            "a derived spellfix projection, rebuilt in Group 6",
        ),
    ];
    entries.sort();
    entries
}

fn media_request_repoints(snapshot: &MergeCatalogSnapshot) -> Vec<MediaRequestRepoint> {
    // OQ10: the request history follows the content into the destination
    // library, and the repoint is named in the preview rather than left for a
    // user to discover through their library permissions.
    let (Some(previous), Some(destination)) = (
        snapshot.source_library_id.as_ref(),
        snapshot.destination_library_id.as_ref(),
    ) else {
        return Vec::new();
    };
    if previous == destination {
        return Vec::new();
    }
    snapshot
        .media_request_ids
        .iter()
        .map(|request_id| MediaRequestRepoint {
            request_id: request_id.clone(),
            previous_library_id: previous.clone(),
            destination_library_id: destination.clone(),
        })
        .collect()
}

fn dropped_categories(snapshot: &MergeCatalogSnapshot) -> Vec<DroppedCategory> {
    let mut dropped = vec![
        // OQ5: the delay queue and its decisions were computed against the
        // source title's quality profile, which FR-063 has just replaced. They
        // re-derive from wanted_items on the next convergence pass.
        DroppedCategory {
            table: "pending_releases".to_string(),
            source_row_count: snapshot.rows("pending_releases"),
            decision: "OQ5".to_string(),
            reason: "the delay queue was chosen against the source title's quality profile, which \
                     the destination's replaces; it re-derives from wanted_items on the next \
                     convergence pass"
                .to_string(),
        },
        DroppedCategory {
            table: "release_decisions".to_string(),
            source_row_count: snapshot.rows("release_decisions"),
            decision: "OQ5".to_string(),
            reason: "decisions made against the source profile are misleading once the \
                     destination's configuration wins"
                .to_string(),
        },
        // OQ4: `episode_set:b3:<hex>` is a BLAKE3 hash over the sorted episode
        // id list and cannot be recomputed against mapped ids, so all five
        // scope_key forms are dropped uniformly rather than leaving season-pack
        // coverage inconsistent with episode coverage.
        DroppedCategory {
            table: "scope_indexer_coverage".to_string(),
            source_row_count: snapshot.rows("scope_indexer_coverage"),
            decision: "OQ4".to_string(),
            reason: "the episode_set:b3: scope key is an irreversible BLAKE3 hash, so every scope \
                     row for the source title is dropped uniformly; coverage re-accumulates over \
                     one search sweep per indexer"
                .to_string(),
        },
        DroppedCategory {
            table: "indexer_search_runs".to_string(),
            source_row_count: snapshot.rows("indexer_search_runs"),
            decision: "OQ4".to_string(),
            reason: "same scope_key encoding as scope_indexer_coverage".to_string(),
        },
        DroppedCategory {
            table: "history_events".to_string(),
            source_row_count: snapshot.rows("history_events"),
            decision: "inventory §5".to_string(),
            reason: "legacy: no production reads or inserts survive".to_string(),
        },
        DroppedCategory {
            table: "title_search_terms".to_string(),
            source_row_count: snapshot.rows("title_search_terms"),
            decision: "inventory §3".to_string(),
            reason: "a derived spellfix projection, cheaper and safer to regenerate (Group 6)"
                .to_string(),
        },
    ];
    dropped.sort();
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::merge::MergedMediaRole;
    use crate::location::merge::map::MergeBlockReason;
    use scryer_domain::EpisodeType;
    use std::collections::{BTreeMap, BTreeSet};

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
            source_tags: vec![
                "scryer:quality-profile:source-profile".to_string(),
                "rewatch".to_string(),
            ],
            destination_tags: vec!["scryer:quality-profile:destination-profile".to_string()],
            media_request_ids: vec!["request-1".to_string()],
            source_row_counts: BTreeMap::from([
                ("media_files".to_string(), 3),
                ("pending_releases".to_string(), 2),
            ]),
            ..MergeCatalogSnapshot::default()
        }
    }

    #[test]
    fn a_clean_snapshot_plans_a_complete_merge() {
        let plan = plan_merge(&snapshot());
        assert!(!plan.is_blocked());
        let map = plan.require_identity_map().expect("the map exists");
        assert_eq!(map.episode("s-e1"), Some("d-e1"));
        // FR-070: the demotion is in the plan and in the summary.
        assert_eq!(plan.role_plan.demotion_count(), 1);
        assert_eq!(plan.summary.role_changes.len(), 1);
        // OQ9: the differing quality profile is an explicit conflict.
        assert_eq!(plan.summary.reserved_tag_conflicts.len(), 1);
        assert_eq!(plan.summary.free_form_tags_added, vec!["rewatch".to_string()]);
        // OQ10: the cross-library repoint is named.
        assert_eq!(plan.summary.media_request_repoints.len(), 1);
        assert_eq!(
            plan.summary.media_request_repoints[0].destination_library_id,
            "library-b"
        );
        // Group 6 is returned, never executed here.
        assert!(
            plan.summary
                .post_merge_work
                .contains(&PostMergeWork::DropSourceIndexerCoverage)
        );
    }

    #[test]
    fn a_same_library_merge_repoints_no_requests() {
        let mut snapshot = snapshot();
        snapshot.destination_library_id = snapshot.source_library_id.clone();
        assert!(plan_merge(&snapshot).summary.media_request_repoints.is_empty());
    }

    #[test]
    fn an_unmapped_episode_blocks_the_plan_and_the_execution() {
        let mut snapshot = snapshot();
        snapshot.source_episodes.push(episode("s-e2", "1", "2"));
        snapshot.episode_references.insert(
            "wanted_items".to_string(),
            BTreeSet::from(["s-e2".to_string()]),
        );
        let plan = plan_merge(&snapshot);
        assert!(plan.is_blocked());
        assert!(plan.identity_map.is_none());
        assert!(
            plan.blocked()
                .iter()
                .any(|record| record.table == "wanted_items"
                    && record.reason == MergeBlockReason::UnmappedEpisode)
        );
        assert!(plan.require_identity_map().is_err());
        // No role plan is computed for a blocked merge.
        assert!(plan.role_plan.rows.is_empty());
    }

    #[test]
    fn the_drop_list_carries_the_oq_that_decided_each_category() {
        let plan = plan_merge(&snapshot());
        let pending = plan
            .summary
            .dropped
            .iter()
            .find(|entry| entry.table == "pending_releases")
            .expect("OQ5 drops the delay queue");
        assert_eq!(pending.decision, "OQ5");
        assert_eq!(pending.source_row_count, 2);
        let coverage = plan
            .summary
            .dropped
            .iter()
            .find(|entry| entry.table == "scope_indexer_coverage")
            .expect("OQ4 drops scope coverage");
        assert_eq!(coverage.decision, "OQ4");
    }

    #[test]
    fn the_summary_states_the_live_schema_deviations() {
        let plan = plan_merge(&snapshot());
        for (table, _) in INVENTORY_DEVIATIONS {
            assert!(
                plan.summary
                    .notes
                    .iter()
                    .any(|note| note.contains(&format!("`{table}`"))),
                "the preview should state the {table} deviation"
            );
        }
    }

    #[test]
    fn the_gate_lists_name_only_tables_the_live_schema_still_has() {
        // `merge-inventory.md` §8 lists `title_history` and `subtitle_blacklist`;
        // both are gone from the live schema, so asserting on them would make
        // the gate itself fail.
        for (table, _, _) in FR067_NO_FK_ASSERTIONS
            .iter()
            .chain(FR067_SET_NULL_ASSERTIONS.iter())
        {
            assert!(
                !matches!(
                    *table,
                    "title_history" | "subtitle_blacklist" | "releases" | "policy_decisions"
                ),
                "{table} is not in the live schema"
            );
        }
    }

    #[test]
    fn media_files_counts_reach_the_disposition_list() {
        let plan = plan_merge(&snapshot());
        let media_files = plan
            .summary
            .dispositions
            .iter()
            .find(|entry| entry.table == "media_files")
            .expect("media_files is inventoried");
        assert_eq!(media_files.disposition, MergeDisposition::Union);
        assert_eq!(media_files.source_row_count, 3);
    }
}
