//! Media-role resolution across a merge (T086, FR-068–FR-070).
//!
//! The role of a media file is a property of a **logical slot**, not of a
//! filename. For episodic content that slot is a mapped destination episode and
//! the enforcement point is real: `idx_file_episode_map_one_primary_per_episode`
//! is a partial unique index on `(episode_id) WHERE role = 'primary'`, present
//! and identically defined on both engines (sqlite migration 0158, postgres
//! migration 0158). A source row arriving as `primary` for an episode the
//! destination already covers *violates that index*, so resolution is not a
//! nicety — an unresolved plan fails the transaction.
//!
//! The three rules, and where each falls out:
//!
//! | Rule | Behavior here |
//! |---|---|
//! | FR-068 | A destination primary keeps `primary`; the incoming primary is demoted to `additional`. An incoming primary for a slot with no destination primary stays `primary`. |
//! | FR-069 | Nothing special is needed: the index is per-episode, so one `file_id` can hold `primary` for an uncovered episode and `additional` for a covered one in the same plan. |
//! | FR-070 | No destination row is ever rewritten — [`resolve_media_roles`] only ever emits rows for source files — and every source role change is recorded in [`MergedRolePlan::role_changes`] for the preview. |
//!
//! # The collapse case
//!
//! Two source episodes can map onto one destination episode only when the
//! identity map allowed it, which [`super::map`] does not: it blocks an
//! ambiguous source identity. The collapse handling here is therefore a
//! belt-and-braces path, and it is still deterministic — the lowest source
//! episode id wins the primary claim — because the `(file_id, episode_id)`
//! primary key would otherwise reject the second row.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::location::merge::MergedMediaRole;
use crate::location::merge::map::MergeIdentityMap;

/// One `file_episode_map` row, from either side.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FileEpisodeRoleRow {
    pub file_id: String,
    pub episode_id: String,
    pub role: MergedMediaRole,
    pub is_filler: bool,
}

/// Why a source file's role changed. Every variant is a preview line item
/// (FR-070).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleChangeReason {
    /// The destination already had a primary for this slot, and FR-070 forbids
    /// demoting it, so the incoming file becomes an additional.
    DestinationPrimaryRetained,
    /// Two source rows claimed primary for the same destination episode; the
    /// first by source episode id kept it.
    SourcePrimaryAlreadyClaimed,
    /// Two source episodes collapsed onto one destination episode for the same
    /// file, so their two rows became one.
    CollapsedSourceEpisodes,
}

impl RoleChangeReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DestinationPrimaryRetained => "destination_primary_retained",
            Self::SourcePrimaryAlreadyClaimed => "source_primary_already_claimed",
            Self::CollapsedSourceEpisodes => "collapsed_source_episodes",
        }
    }
}

/// One media file whose role belongs to the *title's* slot rather than to an
/// episode's — a movie's file, or a title-level extra with no episode mapping.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TitleSlotFileRow {
    pub file_id: String,
    pub role: MergedMediaRole,
}

/// One role change, as the FR-071 preview renders it.
///
/// The episode ids are `None` for a title-slot file: a movie has one slot and
/// it is the title.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MediaRoleChange {
    pub file_id: String,
    #[serde(default)]
    pub source_episode_id: Option<String>,
    #[serde(default)]
    pub destination_episode_id: Option<String>,
    pub previous_role: MergedMediaRole,
    pub new_role: MergedMediaRole,
    pub reason: RoleChangeReason,
}

impl MediaRoleChange {
    /// The sentence FR-070 asks the preview to show for this change. Phrased
    /// once, here, so the plan item, the GraphQL payload, and any log line
    /// cannot describe the same demotion three different ways.
    pub fn describe(&self) -> String {
        let why = match self.reason {
            RoleChangeReason::DestinationPrimaryRetained => {
                "the destination already has a primary for that slot, and no destination primary is demoted by a move"
            }
            RoleChangeReason::SourcePrimaryAlreadyClaimed => {
                "another source file already claimed primary for that destination episode"
            }
            RoleChangeReason::CollapsedSourceEpisodes => {
                "two source episodes map onto that one destination episode, so their rows collapse into one"
            }
        };
        match (
            self.destination_episode_id.as_deref(),
            self.source_episode_id.as_deref(),
        ) {
            (Some(destination), Some(source)) => format!(
                "file {} becomes {} for episode {destination} (was {} for source episode \
                 {source}): {why}",
                self.file_id,
                self.new_role.as_str(),
                self.previous_role.as_str(),
            ),
            _ => format!(
                "file {} becomes {} for the merged title (was {}): {why}",
                self.file_id,
                self.new_role.as_str(),
                self.previous_role.as_str(),
            ),
        }
    }
}

/// The rows Group 1 writes, plus everything the preview has to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedRolePlan {
    /// The post-merge `file_episode_map` rows for the **source** files, keyed
    /// on destination episode ids. Destination rows are untouched and are not
    /// listed.
    pub rows: Vec<FileEpisodeRoleRow>,
    /// The post-merge `media_files.role` values for source files that hang off
    /// the title itself rather than off an episode.
    pub title_slot_rows: Vec<TitleSlotFileRow>,
    pub role_changes: Vec<MediaRoleChange>,
    /// Destination episodes that had no primary and gained one from the source
    /// (FR-068's "stays or becomes primary where none exists").
    pub newly_covered_episodes: BTreeSet<String>,
    /// Source rows whose episode id is not in the map. Non-empty only if the
    /// caller ran resolution against a map that never should have passed
    /// Group 0; the executor treats it as a hard error.
    pub unmapped_rows: Vec<FileEpisodeRoleRow>,
}

impl MergedRolePlan {
    pub fn demotion_count(&self) -> usize {
        self.role_changes
            .iter()
            .filter(|change| {
                change.previous_role == MergedMediaRole::Primary
                    && change.new_role == MergedMediaRole::Additional
            })
            .count()
    }
}

/// Resolve post-merge roles for every source `file_episode_map` row.
///
/// `destination_rows` is read only to learn which destination episodes already
/// have a primary; it is never rewritten, which is the mechanical form of
/// FR-070.
pub fn resolve_media_roles(
    map: &MergeIdentityMap,
    source_rows: &[FileEpisodeRoleRow],
    destination_rows: &[FileEpisodeRoleRow],
    source_title_slot_files: &[TitleSlotFileRow],
    destination_title_slot_has_primary: bool,
) -> MergedRolePlan {
    let destination_primaries: BTreeSet<&str> = destination_rows
        .iter()
        .filter(|row| row.role == MergedMediaRole::Primary)
        .map(|row| row.episode_id.as_str())
        .collect();

    // Deterministic order: the same catalog always produces the same plan, so a
    // preview and its execution agree and a resumed operation repeats itself.
    let mut ordered: Vec<&FileEpisodeRoleRow> = source_rows.iter().collect();
    ordered.sort();

    let mut plan = MergedRolePlan::default();
    // Destination episode → the source row index already holding primary.
    let mut claimed_primary: BTreeMap<String, usize> = BTreeMap::new();
    // (file_id, destination episode) → index into `plan.rows`, so a collapse
    // merges into the row that is already there instead of violating the PK.
    let mut placed: BTreeMap<(String, String), usize> = BTreeMap::new();

    for row in ordered {
        let Some(destination_episode_id) = map.episode(&row.episode_id) else {
            plan.unmapped_rows.push(row.clone());
            continue;
        };
        let destination_episode_id = destination_episode_id.to_string();
        let key = (row.file_id.clone(), destination_episode_id.clone());

        if let Some(&existing) = placed.get(&key) {
            // Same file, two source episodes, one destination episode. Keep the
            // stronger role and record the collapse.
            let existing_role = plan.rows[existing].role;
            let promoted = row.role == MergedMediaRole::Primary
                && existing_role == MergedMediaRole::Additional
                && !destination_primaries.contains(destination_episode_id.as_str())
                && !claimed_primary.contains_key(&destination_episode_id);
            if promoted {
                plan.rows[existing].role = MergedMediaRole::Primary;
                claimed_primary.insert(destination_episode_id.clone(), existing);
                plan.newly_covered_episodes
                    .insert(destination_episode_id.clone());
            }
            plan.rows[existing].is_filler |= row.is_filler;
            plan.role_changes.push(MediaRoleChange {
                file_id: row.file_id.clone(),
                source_episode_id: Some(row.episode_id.clone()),
                destination_episode_id: Some(destination_episode_id),
                previous_role: row.role,
                new_role: plan.rows[existing].role,
                reason: RoleChangeReason::CollapsedSourceEpisodes,
            });
            continue;
        }

        let resolved_role = if row.role == MergedMediaRole::Additional {
            // FR-068: incoming and existing additionals stay additional.
            MergedMediaRole::Additional
        } else if destination_primaries.contains(destination_episode_id.as_str()) {
            // FR-068/FR-070: the destination primary stands.
            plan.role_changes.push(MediaRoleChange {
                file_id: row.file_id.clone(),
                source_episode_id: Some(row.episode_id.clone()),
                destination_episode_id: Some(destination_episode_id.clone()),
                previous_role: MergedMediaRole::Primary,
                new_role: MergedMediaRole::Additional,
                reason: RoleChangeReason::DestinationPrimaryRetained,
            });
            MergedMediaRole::Additional
        } else if claimed_primary.contains_key(&destination_episode_id) {
            plan.role_changes.push(MediaRoleChange {
                file_id: row.file_id.clone(),
                source_episode_id: Some(row.episode_id.clone()),
                destination_episode_id: Some(destination_episode_id.clone()),
                previous_role: MergedMediaRole::Primary,
                new_role: MergedMediaRole::Additional,
                reason: RoleChangeReason::SourcePrimaryAlreadyClaimed,
            });
            MergedMediaRole::Additional
        } else {
            // FR-068's uncovered slot: the incoming primary fills it.
            claimed_primary.insert(destination_episode_id.clone(), plan.rows.len());
            plan.newly_covered_episodes
                .insert(destination_episode_id.clone());
            MergedMediaRole::Primary
        };

        placed.insert(key, plan.rows.len());
        plan.rows.push(FileEpisodeRoleRow {
            file_id: row.file_id.clone(),
            episode_id: destination_episode_id,
            role: resolved_role,
            is_filler: row.is_filler,
        });
    }

    resolve_title_slot_roles(
        source_title_slot_files,
        destination_title_slot_has_primary,
        &mut plan,
    );

    plan
}

/// FR-068 for the slot a movie has: the title itself.
///
/// A movie's file hangs off no episode, so `file_episode_map` says nothing about
/// it and `media_files.role` is the whole story. The rule is the same one the
/// episode pass applies — the destination's primary is never demoted, and an
/// incoming primary fills a slot that has none.
fn resolve_title_slot_roles(
    source_files: &[TitleSlotFileRow],
    destination_has_primary: bool,
    plan: &mut MergedRolePlan,
) {
    let mut ordered: Vec<&TitleSlotFileRow> = source_files.iter().collect();
    ordered.sort();

    let mut primary_claimed = destination_has_primary;
    for file in ordered {
        let resolved = if file.role == MergedMediaRole::Additional {
            MergedMediaRole::Additional
        } else if primary_claimed {
            plan.role_changes.push(MediaRoleChange {
                file_id: file.file_id.clone(),
                source_episode_id: None,
                destination_episode_id: None,
                previous_role: MergedMediaRole::Primary,
                new_role: MergedMediaRole::Additional,
                reason: if destination_has_primary {
                    RoleChangeReason::DestinationPrimaryRetained
                } else {
                    RoleChangeReason::SourcePrimaryAlreadyClaimed
                },
            });
            MergedMediaRole::Additional
        } else {
            primary_claimed = true;
            MergedMediaRole::Primary
        };
        plan.title_slot_rows.push(TitleSlotFileRow {
            file_id: file.file_id.clone(),
            role: resolved,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> MergeIdentityMap {
        MergeIdentityMap {
            source_title_id: "source".to_string(),
            destination_title_id: "destination".to_string(),
            episodes: pairs
                .iter()
                .map(|(source, destination)| (source.to_string(), destination.to_string()))
                .collect(),
            ..MergeIdentityMap::default()
        }
    }

    fn row(file_id: &str, episode_id: &str, role: MergedMediaRole) -> FileEpisodeRoleRow {
        FileEpisodeRoleRow {
            file_id: file_id.to_string(),
            episode_id: episode_id.to_string(),
            role,
            is_filler: false,
        }
    }

    #[test]
    fn an_incoming_primary_fills_a_slot_with_no_destination_primary() {
        let plan = resolve_media_roles(
            &map(&[("s-e1", "d-e1")]),
            &[row("file-in", "s-e1", MergedMediaRole::Primary)],
            &[],
            &[],
            false,
        );
        assert_eq!(plan.rows, vec![row("file-in", "d-e1", MergedMediaRole::Primary)]);
        assert!(plan.role_changes.is_empty());
        assert_eq!(
            plan.newly_covered_episodes,
            BTreeSet::from(["d-e1".to_string()])
        );
    }

    #[test]
    fn a_destination_primary_is_never_demoted_and_the_incoming_one_becomes_additional() {
        let plan = resolve_media_roles(
            &map(&[("s-e1", "d-e1")]),
            &[row("file-in", "s-e1", MergedMediaRole::Primary)],
            &[row("file-dest", "d-e1", MergedMediaRole::Primary)],
            &[],
            false,
        );
        assert_eq!(
            plan.rows,
            vec![row("file-in", "d-e1", MergedMediaRole::Additional)]
        );
        assert_eq!(plan.role_changes.len(), 1);
        let change = &plan.role_changes[0];
        assert_eq!(change.file_id, "file-in");
        assert_eq!(change.previous_role, MergedMediaRole::Primary);
        assert_eq!(change.new_role, MergedMediaRole::Additional);
        assert_eq!(change.reason, RoleChangeReason::DestinationPrimaryRetained);
        assert_eq!(plan.demotion_count(), 1);
        // FR-070: nothing in the plan touches the destination row.
        assert!(plan.rows.iter().all(|row| row.file_id != "file-dest"));
    }

    #[test]
    fn one_multi_episode_file_is_primary_where_uncovered_and_additional_where_covered() {
        // FR-069, and the reason it needs no special case: the partial unique
        // index is per-episode.
        let plan = resolve_media_roles(
            &map(&[("s-e1", "d-e1"), ("s-e2", "d-e2")]),
            &[
                row("file-in", "s-e1", MergedMediaRole::Primary),
                row("file-in", "s-e2", MergedMediaRole::Primary),
            ],
            &[row("file-dest", "d-e1", MergedMediaRole::Primary)],
            &[],
            false,
        );
        assert_eq!(
            plan.rows,
            vec![
                row("file-in", "d-e1", MergedMediaRole::Additional),
                row("file-in", "d-e2", MergedMediaRole::Primary),
            ]
        );
        assert_eq!(plan.role_changes.len(), 1);
        assert_eq!(
            plan.role_changes[0].destination_episode_id.as_deref(),
            Some("d-e1")
        );
    }

    #[test]
    fn an_incoming_additional_stays_additional() {
        let plan = resolve_media_roles(
            &map(&[("s-e1", "d-e1")]),
            &[row("file-in", "s-e1", MergedMediaRole::Additional)],
            &[],
            &[],
            false,
        );
        assert_eq!(
            plan.rows,
            vec![row("file-in", "d-e1", MergedMediaRole::Additional)]
        );
        assert!(plan.role_changes.is_empty());
        // An additional never covers a slot.
        assert!(plan.newly_covered_episodes.is_empty());
    }

    #[test]
    fn two_source_files_claiming_one_uncovered_slot_resolve_deterministically() {
        let plan = resolve_media_roles(
            &map(&[("s-e1", "d-e1"), ("s-e2", "d-e1")]),
            &[
                row("file-b", "s-e2", MergedMediaRole::Primary),
                row("file-a", "s-e1", MergedMediaRole::Primary),
            ],
            &[],
            &[],
            false,
        );
        // Sorted by (file_id, episode_id): file-a wins the primary claim.
        assert_eq!(
            plan.rows,
            vec![
                row("file-a", "d-e1", MergedMediaRole::Primary),
                row("file-b", "d-e1", MergedMediaRole::Additional),
            ]
        );
        assert_eq!(
            plan.role_changes[0].reason,
            RoleChangeReason::SourcePrimaryAlreadyClaimed
        );
    }

    #[test]
    fn two_source_episodes_collapsing_onto_one_slot_merge_into_a_single_row() {
        let mut filler = row("file-in", "s-e2", MergedMediaRole::Primary);
        filler.is_filler = true;
        let plan = resolve_media_roles(
            &map(&[("s-e1", "d-e1"), ("s-e2", "d-e1")]),
            &[row("file-in", "s-e1", MergedMediaRole::Additional), filler],
            &[],
            &[],
            false,
        );
        assert_eq!(plan.rows.len(), 1);
        assert_eq!(plan.rows[0].role, MergedMediaRole::Primary);
        assert!(plan.rows[0].is_filler);
        assert_eq!(
            plan.role_changes[0].reason,
            RoleChangeReason::CollapsedSourceEpisodes
        );
    }

    #[test]
    fn a_movie_file_becomes_additional_when_the_destination_already_has_a_primary() {
        let plan = resolve_media_roles(
            &map(&[]),
            &[],
            &[],
            &[TitleSlotFileRow {
                file_id: "movie-in".to_string(),
                role: MergedMediaRole::Primary,
            }],
            true,
        );
        assert_eq!(
            plan.title_slot_rows,
            vec![TitleSlotFileRow {
                file_id: "movie-in".to_string(),
                role: MergedMediaRole::Additional,
            }]
        );
        assert_eq!(plan.demotion_count(), 1);
        assert_eq!(plan.role_changes[0].destination_episode_id, None);
        assert_eq!(
            plan.role_changes[0].reason,
            RoleChangeReason::DestinationPrimaryRetained
        );
    }

    #[test]
    fn a_movie_file_stays_primary_when_the_destination_has_none() {
        let plan = resolve_media_roles(
            &map(&[]),
            &[],
            &[],
            &[TitleSlotFileRow {
                file_id: "movie-in".to_string(),
                role: MergedMediaRole::Primary,
            }],
            false,
        );
        assert_eq!(
            plan.title_slot_rows,
            vec![TitleSlotFileRow {
                file_id: "movie-in".to_string(),
                role: MergedMediaRole::Primary,
            }]
        );
        assert!(plan.role_changes.is_empty());
    }

    #[test]
    fn only_one_incoming_movie_file_can_claim_the_empty_title_slot() {
        let plan = resolve_media_roles(
            &map(&[]),
            &[],
            &[],
            &[
                TitleSlotFileRow {
                    file_id: "movie-b".to_string(),
                    role: MergedMediaRole::Primary,
                },
                TitleSlotFileRow {
                    file_id: "movie-a".to_string(),
                    role: MergedMediaRole::Primary,
                },
            ],
            false,
        );
        assert_eq!(
            plan.title_slot_rows,
            vec![
                TitleSlotFileRow {
                    file_id: "movie-a".to_string(),
                    role: MergedMediaRole::Primary,
                },
                TitleSlotFileRow {
                    file_id: "movie-b".to_string(),
                    role: MergedMediaRole::Additional,
                },
            ]
        );
        assert_eq!(
            plan.role_changes[0].reason,
            RoleChangeReason::SourcePrimaryAlreadyClaimed
        );
    }

    #[test]
    fn a_row_outside_the_map_is_reported_rather_than_silently_dropped() {
        let plan = resolve_media_roles(
            &map(&[("s-e1", "d-e1")]),
            &[row("file-in", "s-e9", MergedMediaRole::Primary)],
            &[],
            &[],
            false,
        );
        assert!(plan.rows.is_empty());
        assert_eq!(plan.unmapped_rows.len(), 1);
        assert_eq!(plan.unmapped_rows[0].episode_id, "s-e9");
    }
}
