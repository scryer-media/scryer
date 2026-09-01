//! The merge preview summary (FR-071) and the `titles.tags` partition (OQ9).
//!
//! FR-071 asks the preview to state four things: which destination settings
//! win, which source values carry forward, which data is unioned, and which
//! values are dropped or converted. [`MergePreviewSummary`] is that statement,
//! as data rather than as prose, so T087's web work renders it without knowing
//! anything about the table inventory.
//!
//! # `titles.tags` is not a plain tag set (OQ9)
//!
//! `titles.tags` is a JSON array carrying free-form user tags **and** a
//! reserved `scryer:` namespace that stores per-title configuration — the
//! quality profile, the monitor type, the filler and recap policies, the
//! season-folder layout. Every reader in the codebase resolves those with
//! `find_map(strip_prefix(...))`, so first match wins. A naive set-union would
//! leave the merged title carrying two `scryer:quality-profile:` tags and
//! resolve FR-063's own settings by array order.
//!
//! So the partition is explicit: free-form tags union, reserved tags are
//! destination-wins, and **every reserved tag whose two sides disagree becomes
//! a [`ReservedTagConflict`]** rather than a silent drop. See
//! [`partition_tags`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::location::merge::MergeDisposition;
use crate::location::merge::map::MergeBlockedRecord;
use crate::location::merge::roles::MediaRoleChange;

/// The reserved namespace inside `titles.tags`. Anything with this prefix is
/// configuration, not a user tag.
pub const RESERVED_TAG_NAMESPACE: &str = "scryer:";

/// The reserved prefixes confirmed in `merge-inventory.md` §3, each with the
/// setting it carries. Used to name a conflict in the preview; the partition
/// itself keys on [`RESERVED_TAG_NAMESPACE`] so an unlisted reserved tag is
/// still handled destination-wins rather than unioned.
pub const RESERVED_TAG_PREFIXES: &[(&str, &str)] = &[
    ("scryer:quality-profile:", "quality profile"),
    ("scryer:monitor-type:", "monitoring mode"),
    ("scryer:monitor-specials:", "specials monitoring"),
    ("scryer:filler-policy:", "filler handling"),
    ("scryer:recap-policy:", "recap handling"),
    ("scryer:inter-season-movies:", "series-movie inclusion"),
    ("scryer:season-folder:", "season-folder layout"),
    ("scryer:root-folder:", "legacy root assignment"),
    ("scryer:mal-score:", "MyAnimeList score (metadata-derived)"),
    ("scryer:anime-media-type:", "anime media type (metadata-derived)"),
    ("scryer:anime-status:", "anime status (metadata-derived)"),
];

/// One reserved setting whose source and destination values differ. The
/// destination's wins (FR-063); this row is how the preview says so.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReservedTagConflict {
    /// The reserved prefix, e.g. `scryer:quality-profile:`.
    pub prefix: String,
    /// Human label for the setting, when the prefix is a known one.
    pub setting: Option<String>,
    /// The value the destination keeps (the suffix after the prefix).
    pub destination_value: Option<String>,
    /// The value the source loses.
    pub source_value: Option<String>,
}

/// The result of partitioning both titles' tags (OQ9).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagMergeResult {
    /// The merged `titles.tags` array for the destination title, in a stable
    /// order: the destination's tags as they were, then the source's new
    /// free-form tags.
    pub merged_tags: Vec<String>,
    /// Free-form tags the source contributed that the destination did not have.
    pub free_form_tags_added: Vec<String>,
    /// Reserved tags dropped because the destination already had that setting
    /// with the same value — no conflict, no line item beyond the count.
    pub reserved_tags_dropped: Vec<String>,
    /// Reserved settings where the two sides disagreed (FR-071).
    pub reserved_tag_conflicts: Vec<ReservedTagConflict>,
}

/// Partition and merge two titles' tag arrays.
///
/// OQ9: free-form tags union (deduped, case-insensitively, keeping the
/// destination's spelling); reserved `scryer:*` tags are destination-wins, and
/// a differing value becomes an explicit conflict entry.
pub fn partition_tags(source_tags: &[String], destination_tags: &[String]) -> TagMergeResult {
    let mut result = TagMergeResult::default();

    let mut destination_reserved: BTreeMap<String, String> = BTreeMap::new();
    let mut destination_free_form: Vec<String> = Vec::new();
    let mut destination_free_form_folded: Vec<String> = Vec::new();

    for tag in destination_tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        result.merged_tags.push(trimmed.to_string());
        match reserved_split(trimmed) {
            Some((prefix, value)) => {
                // First match wins, exactly as every reader resolves it.
                destination_reserved
                    .entry(prefix)
                    .or_insert_with(|| value.to_string());
            }
            None => {
                let folded = trimmed.to_ascii_lowercase();
                if !destination_free_form_folded.contains(&folded) {
                    destination_free_form_folded.push(folded);
                    destination_free_form.push(trimmed.to_string());
                }
            }
        }
    }

    let mut seen_source_reserved: BTreeMap<String, String> = BTreeMap::new();
    for tag in source_tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        match reserved_split(trimmed) {
            Some((prefix, value)) => {
                if seen_source_reserved.contains_key(&prefix) {
                    // A second source tag for one setting is already
                    // unreachable to every reader; drop it silently.
                    result.reserved_tags_dropped.push(trimmed.to_string());
                    continue;
                }
                seen_source_reserved.insert(prefix.clone(), value.to_string());
                match destination_reserved.get(&prefix) {
                    Some(destination_value) if destination_value == value => {
                        result.reserved_tags_dropped.push(trimmed.to_string());
                    }
                    Some(destination_value) => {
                        result.reserved_tag_conflicts.push(ReservedTagConflict {
                            prefix: prefix.clone(),
                            setting: reserved_setting_label(&prefix),
                            destination_value: Some(destination_value.clone()),
                            source_value: Some(value.to_string()),
                        });
                    }
                    None => {
                        // The destination has no opinion on this setting. FR-063
                        // still gives it the destination's configuration, so the
                        // source's value is dropped — but a setting that
                        // disappears is a conversion the user must see.
                        result.reserved_tag_conflicts.push(ReservedTagConflict {
                            prefix: prefix.clone(),
                            setting: reserved_setting_label(&prefix),
                            destination_value: None,
                            source_value: Some(value.to_string()),
                        });
                    }
                }
            }
            None => {
                let folded = trimmed.to_ascii_lowercase();
                if destination_free_form_folded.contains(&folded) {
                    continue;
                }
                destination_free_form_folded.push(folded);
                result.merged_tags.push(trimmed.to_string());
                result.free_form_tags_added.push(trimmed.to_string());
            }
        }
    }

    result
}

fn reserved_split(tag: &str) -> Option<(String, &str)> {
    let rest = tag.strip_prefix(RESERVED_TAG_NAMESPACE)?;
    // The reserved form is `scryer:<setting>:<value>`. A bare `scryer:foo` with
    // no value is still reserved, and its whole remainder is the prefix.
    match rest.split_once(':') {
        Some((setting, value)) => Some((format!("{RESERVED_TAG_NAMESPACE}{setting}:"), value)),
        None => Some((tag.to_string(), "")),
    }
}

fn reserved_setting_label(prefix: &str) -> Option<String> {
    RESERVED_TAG_PREFIXES
        .iter()
        .find(|(known, _)| *known == prefix)
        .map(|(_, label)| (*label).to_string())
}

/// One table's contribution to the merge, with the row count the preview shows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TableDispositionEntry {
    pub table: String,
    pub disposition: MergeDisposition,
    /// Source rows the disposition applies to.
    pub source_row_count: i64,
    /// Why, in one line — the `merge-inventory.md` justification, or the OQ
    /// number that adjudicated it.
    pub note: String,
}

/// Something the merge deliberately does not carry (OQ1, OQ2, OQ4, OQ5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DroppedCategory {
    pub table: String,
    pub source_row_count: i64,
    /// The adjudication that decided it, e.g. `"OQ5"`.
    pub decision: String,
    pub reason: String,
}

/// A `media_requests` row whose `library_id` moves to the destination library
/// (OQ10).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MediaRequestRepoint {
    pub request_id: String,
    pub previous_library_id: String,
    pub destination_library_id: String,
}

/// A destination value that wins under FR-063, named so the preview can say so.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DestinationWinsEntry {
    /// e.g. `"title id"`, `"metadata identity"`, `"quality configuration"`.
    pub setting: String,
    pub destination_value: Option<String>,
    pub source_value: Option<String>,
}

/// Work the caller must schedule after the transaction commits (Group 6). All
/// of it is derived-cache regeneration: a crash between Group 5 and Group 6
/// leaves a correct catalog with a stale index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostMergeWork {
    /// `title_search_terms` reindex for the destination title.
    ReindexTitleSearchTerms,
    /// `title_more_like_this_items_new` / `title_recommendation_cards`.
    RegenerateRecommendations,
    /// Title/library statistics recomputation.
    RecomputeStatistics,
    /// OQ4: drop every `scope_indexer_coverage` / `indexer_search_runs` row for
    /// the source title's five `scope_key` forms. Outside the transaction
    /// because coverage is a derived cache and the `episode_set:b3:` key is
    /// irreversible.
    DropSourceIndexerCoverage,
}

impl PostMergeWork {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReindexTitleSearchTerms => "reindex_title_search_terms",
            Self::RegenerateRecommendations => "regenerate_recommendations",
            Self::RecomputeStatistics => "recompute_statistics",
            Self::DropSourceIndexerCoverage => "drop_source_indexer_coverage",
        }
    }
}

/// The complete FR-071 preview summary. Self-describing on purpose: every
/// consumer (the GraphQL preview, Activity, the confirmation dialog) reads this
/// one structure and never the table inventory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePreviewSummary {
    pub source_title_id: String,
    pub destination_title_id: String,
    pub source_library_id: Option<String>,
    pub destination_library_id: Option<String>,

    /// FR-063: what the destination keeps.
    pub destination_wins: Vec<DestinationWinsEntry>,
    /// FR-064: what carries forward, per table, with counts.
    pub dispositions: Vec<TableDispositionEntry>,
    /// FR-066: non-empty means the merge cannot run.
    pub blocked: Vec<MergeBlockedRecord>,
    /// FR-070: every role change, none of them silent.
    pub role_changes: Vec<MediaRoleChange>,
    /// OQ9: reserved settings whose values disagree.
    pub reserved_tag_conflicts: Vec<ReservedTagConflict>,
    /// OQ9: free-form tags the source contributes.
    pub free_form_tags_added: Vec<String>,
    /// OQ10: requests whose library follows the content.
    pub media_request_repoints: Vec<MediaRequestRepoint>,
    /// OQ1/OQ2/OQ4/OQ5: what is not carried, and why.
    pub dropped: Vec<DroppedCategory>,
    /// Group 6.
    pub post_merge_work: Vec<PostMergeWork>,
    /// Anything else the operator should read before confirming — including the
    /// inventory deviations this schema forced.
    pub notes: Vec<String>,
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

    pub fn rows_by_disposition(&self, disposition: MergeDisposition) -> i64 {
        self.dispositions
            .iter()
            .filter(|entry| entry.disposition == disposition)
            .map(|entry| entry.source_row_count)
            .sum()
    }

    pub fn dropped_row_total(&self) -> i64 {
        self.dropped.iter().map(|entry| entry.source_row_count).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::merge::map::MergeBlockReason;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn free_form_tags_union_and_dedupe_case_insensitively() {
        let result = partition_tags(&tags(&["Anime", "rewatch"]), &tags(&["anime", "4k"]));
        assert_eq!(result.merged_tags, tags(&["anime", "4k", "rewatch"]));
        assert_eq!(result.free_form_tags_added, tags(&["rewatch"]));
        assert!(result.reserved_tag_conflicts.is_empty());
    }

    #[test]
    fn a_differing_reserved_tag_is_an_explicit_conflict_not_a_silent_drop() {
        let result = partition_tags(
            &tags(&["scryer:quality-profile:profile-source"]),
            &tags(&["scryer:quality-profile:profile-destination"]),
        );
        // The destination's array is unchanged: FR-063 keeps its configuration.
        assert_eq!(
            result.merged_tags,
            tags(&["scryer:quality-profile:profile-destination"])
        );
        assert_eq!(result.reserved_tag_conflicts.len(), 1);
        let conflict = &result.reserved_tag_conflicts[0];
        assert_eq!(conflict.prefix, "scryer:quality-profile:");
        assert_eq!(conflict.setting.as_deref(), Some("quality profile"));
        assert_eq!(conflict.source_value.as_deref(), Some("profile-source"));
        assert_eq!(
            conflict.destination_value.as_deref(),
            Some("profile-destination")
        );
    }

    #[test]
    fn an_identical_reserved_tag_is_dropped_without_a_conflict() {
        let result = partition_tags(
            &tags(&["scryer:monitor-type:allepisodes"]),
            &tags(&["scryer:monitor-type:allepisodes"]),
        );
        assert!(result.reserved_tag_conflicts.is_empty());
        assert_eq!(
            result.reserved_tags_dropped,
            tags(&["scryer:monitor-type:allepisodes"])
        );
        assert_eq!(result.merged_tags.len(), 1);
    }

    #[test]
    fn a_source_only_reserved_setting_is_still_a_conflict_because_it_disappears() {
        let result = partition_tags(&tags(&["scryer:filler-policy:skip"]), &tags(&["4k"]));
        assert_eq!(result.merged_tags, tags(&["4k"]));
        let conflict = &result.reserved_tag_conflicts[0];
        assert_eq!(conflict.destination_value, None);
        assert_eq!(conflict.source_value.as_deref(), Some("skip"));
        assert_eq!(conflict.setting.as_deref(), Some("filler handling"));
    }

    #[test]
    fn an_unlisted_reserved_prefix_is_still_destination_wins() {
        let result = partition_tags(
            &tags(&["scryer:some-future-setting:a"]),
            &tags(&["scryer:some-future-setting:b"]),
        );
        assert_eq!(result.free_form_tags_added, Vec::<String>::new());
        assert_eq!(result.reserved_tag_conflicts.len(), 1);
        assert_eq!(result.reserved_tag_conflicts[0].setting, None);
    }

    #[test]
    fn a_reserved_tag_never_leaks_into_the_free_form_union() {
        let result = partition_tags(&tags(&["scryer:season-folder:true", "grabbed"]), &tags(&[]));
        assert_eq!(result.merged_tags, tags(&["grabbed"]));
        assert_eq!(result.free_form_tags_added, tags(&["grabbed"]));
    }

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
                    table: "wanted_items".to_string(),
                    reason: MergeBlockReason::UnmappedEpisode,
                    source_id: "e-1".to_string(),
                    detail: "rows cannot be carried".to_string(),
                },
            ],
            ..MergePreviewSummary::default()
        };
        assert!(summary.is_blocked());
        let reason = summary.blocked_reason().expect("a blocked summary has one");
        assert!(reason.contains("episodes (unmapped_episode): e-1"));
        assert!(reason.contains("wanted_items (unmapped_episode): e-1"));
    }

    #[test]
    fn disposition_and_drop_totals_add_up() {
        let summary = MergePreviewSummary {
            dispositions: vec![
                TableDispositionEntry {
                    table: "media_files".to_string(),
                    disposition: MergeDisposition::Union,
                    source_row_count: 3,
                    note: String::new(),
                },
                TableDispositionEntry {
                    table: "blocklist".to_string(),
                    disposition: MergeDisposition::Union,
                    source_row_count: 2,
                    note: String::new(),
                },
                TableDispositionEntry {
                    table: "file_episode_map".to_string(),
                    disposition: MergeDisposition::Map,
                    source_row_count: 5,
                    note: String::new(),
                },
            ],
            dropped: vec![DroppedCategory {
                table: "pending_releases".to_string(),
                source_row_count: 4,
                decision: "OQ5".to_string(),
                reason: "re-derives from wanted_items against the destination profile".to_string(),
            }],
            ..MergePreviewSummary::default()
        };
        assert_eq!(summary.rows_by_disposition(MergeDisposition::Union), 5);
        assert_eq!(summary.rows_by_disposition(MergeDisposition::Map), 5);
        assert_eq!(summary.dropped_row_total(), 4);
    }
}
