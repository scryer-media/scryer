//! Group 0 of the merge: the source→destination identity map, and the FR-066
//! block decision (US7, T085).
//!
//! D8 says the full map is built *before* anything is written, so a title that
//! cannot be mapped costs no rollback. Everything in this module is therefore
//! pure: the caller assembles both sides' catalog shapes (the Group 0 read set
//! in `merge-inventory.md` §8) and the evaluation here is a function over those
//! facts, testable from literals exactly like [`crate::location::identity`].
//!
//! # What "identity" means here
//!
//! An episode is matched on its *logical slot*, never on its row id and never
//! on its filename:
//!
//! | Key | Built from | Used when |
//! |---|---|---|
//! | [`EpisodeIdentityKey::SeasonEpisode`] | `episode_type` + `season_number` + `episode_number` | both numbers parse |
//! | [`EpisodeIdentityKey::Absolute`] | `episode_type` + `absolute_number` | the season/episode key found no candidate |
//!
//! `episode_type` is part of the key on purpose. A `special` S1E1 and a
//! `standard` S1E1 are different slots, and FR-066 is explicit that the merge
//! blocks rather than guesses — so a type disagreement between the two sides
//! surfaces as an unmapped episode rather than as a silent mis-attachment.
//!
//! # Three ways an episode blocks
//!
//! - **Unmapped** — no destination episode carries the identity.
//! - **Ambiguous destination** — more than one destination episode does. Never
//!   resolved by picking one (FR-066).
//! - **Ambiguous source** — two *source* episodes carry the same identity, so
//!   the map would collapse them. That is corrupt source data, and collapsing
//!   would silently merge two slots' history, so it blocks too.
//!
//! An episode with no usable identity at all (no parseable season/episode pair
//! and no absolute number) is [`MergeBlockReason::UnidentifiableEpisode`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use scryer_domain::{CollectionType, EpisodeType};

/// The record types whose unmapped source rows stop the operation
/// (`merge-inventory.md` §7, FR-066).
///
/// Four of the inventory's fourteen entries are absent here and the reason is
/// recorded at [`FR066_BLOCKING_TABLE_OMISSIONS`]: they name tables that the
/// live schema no longer has, or dispositions that the OQ adjudications turned
/// into deletes.
pub const FR066_BLOCKING_TABLES: &[&str] = &[
    "episodes",
    "file_episode_map",
    "episode_external_ids",
    "wanted_items",
    "download_submissions",
    "download_submission_episode_links",
    "download_import_artifacts",
    "subtitle_downloads",
    "domain_events",
    "media_server_playback_items",
    "workflow_operations",
];

/// Why the four remaining `merge-inventory.md` §7 entries are not in
/// [`FR066_BLOCKING_TABLES`]. Kept as data so the FR-071 preview can state it
/// rather than leaving a reader to infer it from an absence.
pub const FR066_BLOCKING_TABLE_OMISSIONS: &[(&str, &str)] = &[
    (
        "releases",
        "dropped from the live schema by migration 0122; OQ1's `drop` has nothing to act on",
    ),
    (
        "policy_decisions",
        "dropped from the live schema by migration 0011; OQ2's `drop` has nothing to act on",
    ),
    (
        "pending_releases",
        "OQ5 deletes the source title's delay-queue rows instead of mapping them, so no record is \
         attached to a guessed identity",
    ),
    (
        "release_decisions",
        "OQ5 deletes the source title's decision rows instead of mapping them",
    ),
];

/// Facts about one episode, from either side of the merge.
///
/// Numbers arrive as the `TEXT` the schema stores them in; parsing is this
/// module's job so a `"01"` and a `"1"` are the same slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeIdentityFacts {
    pub id: String,
    pub episode_type: EpisodeType,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub absolute_number: Option<String>,
    /// The episode's collection, so a mapped episode can be re-parented onto
    /// the destination's collection row.
    pub collection_id: Option<String>,
}

impl EpisodeIdentityFacts {
    /// The preferred key: type + season + episode. `None` when either number is
    /// missing or non-numeric.
    pub fn season_episode_key(&self) -> Option<EpisodeIdentityKey> {
        let season = parse_number(self.season_number.as_deref())?;
        let episode = parse_number(self.episode_number.as_deref())?;
        Some(EpisodeIdentityKey::SeasonEpisode {
            episode_type: self.episode_type.as_str(),
            season,
            episode,
        })
    }

    /// The fallback key for absolute-numbered (anime) catalogs.
    pub fn absolute_key(&self) -> Option<EpisodeIdentityKey> {
        let absolute = parse_number(self.absolute_number.as_deref())?;
        Some(EpisodeIdentityKey::Absolute {
            episode_type: self.episode_type.as_str(),
            absolute,
        })
    }

    fn keys(&self) -> Vec<EpisodeIdentityKey> {
        [self.season_episode_key(), self.absolute_key()]
            .into_iter()
            .flatten()
            .collect()
    }
}

/// One logical episode slot. Ordering is derived so the key can index a
/// `BTreeMap` and so blocked records sort deterministically.
///
/// The episode type is carried as its `as_str()` discriminant rather than as
/// [`EpisodeType`]: the domain enum is deliberately not `Ord`, and this key
/// indexes a `BTreeMap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EpisodeIdentityKey {
    SeasonEpisode {
        episode_type: &'static str,
        season: i64,
        episode: i64,
    },
    Absolute {
        episode_type: &'static str,
        absolute: i64,
    },
}

impl EpisodeIdentityKey {
    pub fn describe(&self) -> String {
        match self {
            Self::SeasonEpisode {
                episode_type,
                season,
                episode,
            } => format!("{episode_type} S{season:02}E{episode:02}"),
            Self::Absolute {
                episode_type,
                absolute,
            } => format!("{episode_type} #{absolute}"),
        }
    }
}

/// Facts about one collection (a season, an arc, a specials bucket).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionIdentityFacts {
    pub id: String,
    pub collection_type: CollectionType,
    /// `collections.collection_index` — the season number for a season.
    pub collection_index: String,
}

impl CollectionIdentityFacts {
    fn key(&self) -> CollectionIdentityKey {
        CollectionIdentityKey {
            collection_type: self.collection_type.as_str(),
            // Season "01" and season "1" are the same season; a non-numeric
            // index (an arc label) falls back to a trimmed, case-folded string.
            index: parse_number(Some(&self.collection_index))
                .map(|value| value.to_string())
                .unwrap_or_else(|| self.collection_index.trim().to_ascii_lowercase()),
        }
    }
}

/// As with [`EpisodeIdentityKey`], the type is carried as its `as_str()`
/// discriminant because [`CollectionType`] is not `Ord`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CollectionIdentityKey {
    collection_type: &'static str,
    index: String,
}

/// Facts about one series-movie link. D8 makes links first-class map entries;
/// the shared `movie_entities` row is the identity, because that entity is not
/// title-owned and survives the merge untouched (`merge-inventory.md` §1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesMovieLinkIdentityFacts {
    pub id: String,
    pub movie_entity_id: String,
    /// `series_movie_links.legacy_collection_id`, `UNIQUE` across the table. A
    /// source link carrying one cannot be repointed while the destination link
    /// carries the same value, so the executor nulls the source's first.
    pub legacy_collection_id: Option<String>,
}

/// Everything Group 0 reads, from both sides, plus the episode-scoped record
/// references FR-066 is evaluated against.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeIdentityInputs {
    pub source_title_id: String,
    pub destination_title_id: String,
    pub source_episodes: Vec<EpisodeIdentityFacts>,
    pub destination_episodes: Vec<EpisodeIdentityFacts>,
    pub source_collections: Vec<CollectionIdentityFacts>,
    pub destination_collections: Vec<CollectionIdentityFacts>,
    pub source_links: Vec<SeriesMovieLinkIdentityFacts>,
    pub destination_links: Vec<SeriesMovieLinkIdentityFacts>,
    /// Table name → the source episode ids that table's source rows reference.
    /// Only tables in [`FR066_BLOCKING_TABLES`] are evaluated; anything else is
    /// reported as an ignored key rather than silently dropped.
    pub episode_references: BTreeMap<String, BTreeSet<String>>,
    /// Ids of location operations that are resumable (not terminal) and hold
    /// the source title. Any entry hard-blocks the merge — OQ7(a).
    pub resumable_operations_holding_source: Vec<String>,
    /// Ids of unconsumed `manual_import_selections` rows on the source title.
    /// `merge-inventory.md` §3 makes these a block, with `map` as the fallback
    /// only if FR-086's active-work gate is ever relaxed.
    pub unconsumed_manual_import_selections: Vec<String>,
}

/// Why one record blocks the merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeBlockReason {
    /// No destination episode carries the source episode's identity.
    UnmappedEpisode,
    /// More than one destination episode carries it.
    AmbiguousDestinationEpisode,
    /// Two source episodes carry the same identity, so the map would collapse
    /// two logical slots into one.
    AmbiguousSourceEpisode,
    /// The source episode has neither a parseable season/episode pair nor an
    /// absolute number, so it has no identity to match on.
    UnidentifiableEpisode,
    /// A record references a source episode id that is not in the catalog.
    UnknownEpisodeReference,
    UnmappedCollection,
    AmbiguousDestinationCollection,
    AmbiguousSourceCollection,
    UnmappedSeriesMovieLink,
    AmbiguousDestinationSeriesMovieLink,
    AmbiguousSourceSeriesMovieLink,
    /// OQ7(a): a resumable location operation still holds the source title.
    ResumableOperationHoldsSource,
    /// An unconsumed manual-import selection is an active import.
    ActiveManualImportSelection,
}

impl MergeBlockReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnmappedEpisode => "unmapped_episode",
            Self::AmbiguousDestinationEpisode => "ambiguous_destination_episode",
            Self::AmbiguousSourceEpisode => "ambiguous_source_episode",
            Self::UnidentifiableEpisode => "unidentifiable_episode",
            Self::UnknownEpisodeReference => "unknown_episode_reference",
            Self::UnmappedCollection => "unmapped_collection",
            Self::AmbiguousDestinationCollection => "ambiguous_destination_collection",
            Self::AmbiguousSourceCollection => "ambiguous_source_collection",
            Self::UnmappedSeriesMovieLink => "unmapped_series_movie_link",
            Self::AmbiguousDestinationSeriesMovieLink => "ambiguous_destination_series_movie_link",
            Self::AmbiguousSourceSeriesMovieLink => "ambiguous_source_series_movie_link",
            Self::ResumableOperationHoldsSource => "resumable_operation_holds_source",
            Self::ActiveManualImportSelection => "active_manual_import_selection",
        }
    }
}

/// One blocked record. FR-066 requires the checkpoint's `blocked_reason` to
/// name the table and the unmapped episode, so both are carried, never folded
/// into prose.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MergeBlockedRecord {
    /// The table whose source row cannot be carried. `episodes` for the map
    /// itself.
    pub table: String,
    pub reason: MergeBlockReason,
    /// The source id that could not be resolved: an episode id, a collection
    /// id, a link id, or an operation id.
    pub source_id: String,
    pub detail: String,
}

impl MergeBlockedRecord {
    /// One line, suitable for `location_operation_title_checkpoints.blocked_reason`.
    pub fn summary_line(&self) -> String {
        format!(
            "{} ({}): {} — {}",
            self.table,
            self.reason.as_str(),
            self.source_id,
            self.detail
        )
    }
}

/// The complete source→destination map. Every value is a destination id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeIdentityMap {
    pub source_title_id: String,
    pub destination_title_id: String,
    pub episodes: BTreeMap<String, String>,
    pub collections: BTreeMap<String, String>,
    pub series_movie_links: BTreeMap<String, String>,
    /// Source link ids whose `legacy_collection_id` must be nulled before the
    /// repoint, because the destination link already holds that `UNIQUE` value.
    pub legacy_collection_ids_to_clear: Vec<String>,
    /// Table names present in [`MergeIdentityInputs::episode_references`] that
    /// are not in [`FR066_BLOCKING_TABLES`]. Reported rather than dropped so a
    /// caller that adds a table without adding it to the blocking set finds
    /// out.
    pub unevaluated_reference_tables: Vec<String>,
}

impl MergeIdentityMap {
    pub fn episode(&self, source_episode_id: &str) -> Option<&str> {
        self.episodes.get(source_episode_id).map(String::as_str)
    }

    pub fn collection(&self, source_collection_id: &str) -> Option<&str> {
        self.collections.get(source_collection_id).map(String::as_str)
    }

    pub fn series_movie_link(&self, source_link_id: &str) -> Option<&str> {
        self.series_movie_links
            .get(source_link_id)
            .map(String::as_str)
    }
}

/// Group 0's verdict: a complete map, or the FR-066 blocked set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum MergeIdentityOutcome {
    Mapped(Box<MergeIdentityMap>),
    Blocked(Vec<MergeBlockedRecord>),
}

impl MergeIdentityOutcome {
    pub fn mapped(&self) -> Option<&MergeIdentityMap> {
        match self {
            Self::Mapped(map) => Some(map),
            Self::Blocked(_) => None,
        }
    }

    pub fn blocked(&self) -> &[MergeBlockedRecord] {
        match self {
            Self::Mapped(_) => &[],
            Self::Blocked(records) => records,
        }
    }
}

/// Build the source→destination identity map, or the blocked set.
///
/// The whole evaluation runs before it decides: a blocked merge reports *every*
/// unmappable record, not just the first, so an operator fixes the catalog once
/// rather than re-running the preview per episode.
pub fn evaluate_identity_map(inputs: &MergeIdentityInputs) -> MergeIdentityOutcome {
    let mut blocked: Vec<MergeBlockedRecord> = Vec::new();

    // OQ7(a): a resumable operation holding the source title is a hard block,
    // in the shape of FR-086's active-work gate. Rewriting its confirmed
    // `plan_json` would falsify what the user confirmed and invalidate
    // `plan_fingerprint` (FR-081/FR-089), so the merge refuses instead.
    for operation_id in &inputs.resumable_operations_holding_source {
        blocked.push(MergeBlockedRecord {
            table: "location_operations".to_string(),
            reason: MergeBlockReason::ResumableOperationHoldsSource,
            source_id: operation_id.clone(),
            detail: format!(
                "location operation {operation_id} is resumable and still holds source title {}; \
                 finish, cancel, or fail it before merging (OQ7)",
                inputs.source_title_id
            ),
        });
    }

    for selection_id in &inputs.unconsumed_manual_import_selections {
        blocked.push(MergeBlockedRecord {
            table: "manual_import_selections".to_string(),
            reason: MergeBlockReason::ActiveManualImportSelection,
            source_id: selection_id.clone(),
            detail: "an unconsumed manual-import selection is an active import on the source title"
                .to_string(),
        });
    }

    let episodes = map_episodes(inputs, &mut blocked);
    let collections = map_collections(inputs, &mut blocked);
    let (series_movie_links, legacy_collection_ids_to_clear) = map_links(inputs, &mut blocked);

    // FR-066 proper: every episode-scoped record type, evaluated against the
    // map that was just built. A reference to an episode id the source catalog
    // does not even contain is its own reason, so a dangling row is never
    // mistaken for a mapping failure.
    let mut unevaluated_reference_tables = Vec::new();
    for (table, episode_ids) in &inputs.episode_references {
        if !FR066_BLOCKING_TABLES.contains(&table.as_str()) {
            unevaluated_reference_tables.push(table.clone());
            continue;
        }
        let known: BTreeSet<&str> = inputs
            .source_episodes
            .iter()
            .map(|episode| episode.id.as_str())
            .collect();
        for episode_id in episode_ids {
            if episodes.contains_key(episode_id) {
                continue;
            }
            let (reason, detail) = if known.contains(episode_id.as_str()) {
                (
                    MergeBlockReason::UnmappedEpisode,
                    format!(
                        "source episode {episode_id} has no destination counterpart, so {table} \
                         rows referencing it cannot be carried"
                    ),
                )
            } else {
                (
                    MergeBlockReason::UnknownEpisodeReference,
                    format!(
                        "{table} references episode {episode_id}, which is not an episode of \
                         source title {}",
                        inputs.source_title_id
                    ),
                )
            };
            blocked.push(MergeBlockedRecord {
                table: table.clone(),
                reason,
                source_id: episode_id.clone(),
                detail,
            });
        }
    }

    if !blocked.is_empty() {
        blocked.sort();
        blocked.dedup();
        return MergeIdentityOutcome::Blocked(blocked);
    }

    unevaluated_reference_tables.sort();
    unevaluated_reference_tables.dedup();

    MergeIdentityOutcome::Mapped(Box::new(MergeIdentityMap {
        source_title_id: inputs.source_title_id.clone(),
        destination_title_id: inputs.destination_title_id.clone(),
        episodes,
        collections,
        series_movie_links,
        legacy_collection_ids_to_clear,
        unevaluated_reference_tables,
    }))
}

fn map_episodes(
    inputs: &MergeIdentityInputs,
    blocked: &mut Vec<MergeBlockedRecord>,
) -> BTreeMap<String, String> {
    // Destination index, per key form. A key with more than one destination
    // episode is ambiguous and can never be used.
    let mut destination_index: BTreeMap<EpisodeIdentityKey, Vec<&str>> = BTreeMap::new();
    for episode in &inputs.destination_episodes {
        for key in episode.keys() {
            destination_index
                .entry(key)
                .or_default()
                .push(episode.id.as_str());
        }
    }

    // Source-side duplicates: two source episodes claiming one slot.
    let mut source_index: BTreeMap<EpisodeIdentityKey, Vec<&str>> = BTreeMap::new();
    for episode in &inputs.source_episodes {
        if let Some(key) = episode.season_episode_key() {
            source_index.entry(key).or_default().push(&episode.id);
        } else if let Some(key) = episode.absolute_key() {
            source_index.entry(key).or_default().push(&episode.id);
        }
    }

    let mut mapped = BTreeMap::new();
    for episode in &inputs.source_episodes {
        let keys = episode.keys();
        if keys.is_empty() {
            blocked.push(MergeBlockedRecord {
                table: "episodes".to_string(),
                reason: MergeBlockReason::UnidentifiableEpisode,
                source_id: episode.id.clone(),
                detail: format!(
                    "source episode {} carries neither a season/episode pair nor an absolute \
                     number, so it has no identity to map",
                    episode.id
                ),
            });
            continue;
        }

        let primary_key = keys[0];
        if source_index
            .get(&primary_key)
            .is_some_and(|ids| ids.len() > 1)
        {
            blocked.push(MergeBlockedRecord {
                table: "episodes".to_string(),
                reason: MergeBlockReason::AmbiguousSourceEpisode,
                source_id: episode.id.clone(),
                detail: format!(
                    "source title has more than one episode at {}; merging would collapse two \
                     logical slots",
                    primary_key.describe()
                ),
            });
            continue;
        }

        let mut resolved: Option<&str> = None;
        let mut ambiguous: Option<EpisodeIdentityKey> = None;
        for key in &keys {
            match destination_index.get(key).map(Vec::as_slice) {
                None | Some([]) => continue,
                Some([only]) => {
                    resolved = Some(only);
                    break;
                }
                Some(_) => {
                    ambiguous = Some(*key);
                    break;
                }
            }
        }

        match (resolved, ambiguous) {
            (Some(destination_id), _) => {
                mapped.insert(episode.id.clone(), destination_id.to_string());
            }
            (None, Some(key)) => blocked.push(MergeBlockedRecord {
                table: "episodes".to_string(),
                reason: MergeBlockReason::AmbiguousDestinationEpisode,
                source_id: episode.id.clone(),
                detail: format!(
                    "more than one destination episode carries {}; FR-066 refuses to guess",
                    key.describe()
                ),
            }),
            (None, None) => blocked.push(MergeBlockedRecord {
                table: "episodes".to_string(),
                reason: MergeBlockReason::UnmappedEpisode,
                source_id: episode.id.clone(),
                detail: format!(
                    "no destination episode carries {}",
                    keys.iter()
                        .map(EpisodeIdentityKey::describe)
                        .collect::<Vec<_>>()
                        .join(" or ")
                ),
            }),
        }
    }

    mapped
}

fn map_collections(
    inputs: &MergeIdentityInputs,
    blocked: &mut Vec<MergeBlockedRecord>,
) -> BTreeMap<String, String> {
    let mut destination_index: BTreeMap<CollectionIdentityKey, Vec<&str>> = BTreeMap::new();
    for collection in &inputs.destination_collections {
        destination_index
            .entry(collection.key())
            .or_default()
            .push(&collection.id);
    }
    let mut source_index: BTreeMap<CollectionIdentityKey, usize> = BTreeMap::new();
    for collection in &inputs.source_collections {
        *source_index.entry(collection.key()).or_default() += 1;
    }

    let mut mapped = BTreeMap::new();
    for collection in &inputs.source_collections {
        let key = collection.key();
        if source_index.get(&key).copied().unwrap_or(0) > 1 {
            blocked.push(MergeBlockedRecord {
                table: "collections".to_string(),
                reason: MergeBlockReason::AmbiguousSourceCollection,
                source_id: collection.id.clone(),
                detail: format!(
                    "source title has more than one {} collection at index {}",
                    key.collection_type,
                    key.index
                ),
            });
            continue;
        }
        match destination_index.get(&key).map(Vec::as_slice) {
            Some([only]) => {
                mapped.insert(collection.id.clone(), only.to_string());
            }
            Some(many) if many.len() > 1 => blocked.push(MergeBlockedRecord {
                table: "collections".to_string(),
                reason: MergeBlockReason::AmbiguousDestinationCollection,
                source_id: collection.id.clone(),
                detail: format!(
                    "{} destination collections carry {} index {}",
                    many.len(),
                    key.collection_type,
                    key.index
                ),
            }),
            _ => blocked.push(MergeBlockedRecord {
                table: "collections".to_string(),
                reason: MergeBlockReason::UnmappedCollection,
                source_id: collection.id.clone(),
                detail: format!(
                    "no destination collection carries {} index {}",
                    key.collection_type,
                    key.index
                ),
            }),
        }
    }

    mapped
}

fn map_links(
    inputs: &MergeIdentityInputs,
    blocked: &mut Vec<MergeBlockedRecord>,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut destination_index: BTreeMap<&str, Vec<&SeriesMovieLinkIdentityFacts>> = BTreeMap::new();
    for link in &inputs.destination_links {
        destination_index
            .entry(link.movie_entity_id.as_str())
            .or_default()
            .push(link);
    }
    let mut source_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for link in &inputs.source_links {
        *source_counts
            .entry(link.movie_entity_id.as_str())
            .or_default() += 1;
    }

    let mut mapped = BTreeMap::new();
    let mut legacy_to_clear = Vec::new();
    for link in &inputs.source_links {
        let entity = link.movie_entity_id.as_str();
        if source_counts.get(entity).copied().unwrap_or(0) > 1 {
            blocked.push(MergeBlockedRecord {
                table: "series_movie_links".to_string(),
                reason: MergeBlockReason::AmbiguousSourceSeriesMovieLink,
                source_id: link.id.clone(),
                detail: format!(
                    "source title has more than one series-movie link to movie entity {entity}"
                ),
            });
            continue;
        }
        match destination_index.get(entity).map(Vec::as_slice) {
            Some([only]) => {
                mapped.insert(link.id.clone(), only.id.clone());
                // `UNIQUE(legacy_collection_id)`: two links cannot carry the
                // same legacy id, so the source's is cleared before the
                // repoint. Keeping the destination's is FR-065.
                if link.legacy_collection_id.is_some()
                    && only.legacy_collection_id.is_some()
                    && link.legacy_collection_id == only.legacy_collection_id
                {
                    legacy_to_clear.push(link.id.clone());
                }
            }
            Some(many) if many.len() > 1 => blocked.push(MergeBlockedRecord {
                table: "series_movie_links".to_string(),
                reason: MergeBlockReason::AmbiguousDestinationSeriesMovieLink,
                source_id: link.id.clone(),
                detail: format!(
                    "{} destination links point at movie entity {entity}",
                    many.len()
                ),
            }),
            _ => blocked.push(MergeBlockedRecord {
                table: "series_movie_links".to_string(),
                reason: MergeBlockReason::UnmappedSeriesMovieLink,
                source_id: link.id.clone(),
                detail: format!("no destination link points at movie entity {entity}"),
            }),
        }
    }

    (mapped, legacy_to_clear)
}

/// `"01"`, `" 1 "`, and `"1"` are one number; `"1a"` and `""` are not numbers.
fn parse_number(value: Option<&str>) -> Option<i64> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn inputs(
        source_episodes: Vec<EpisodeIdentityFacts>,
        destination_episodes: Vec<EpisodeIdentityFacts>,
    ) -> MergeIdentityInputs {
        MergeIdentityInputs {
            source_title_id: "source".to_string(),
            destination_title_id: "destination".to_string(),
            source_episodes,
            destination_episodes,
            ..MergeIdentityInputs::default()
        }
    }

    #[test]
    fn season_and_episode_numbers_map_across_text_padding() {
        let outcome = evaluate_identity_map(&inputs(
            vec![episode("s1", "01", "01"), episode("s2", "1", "2")],
            vec![episode("d1", "1", "1"), episode("d2", " 01 ", "02")],
        ));
        let map = outcome.mapped().expect("both episodes map");
        assert_eq!(map.episode("s1"), Some("d1"));
        assert_eq!(map.episode("s2"), Some("d2"));
    }

    #[test]
    fn an_episode_type_mismatch_blocks_rather_than_guessing() {
        let mut special = episode("s1", "1", "1");
        special.episode_type = EpisodeType::Special;
        let outcome = evaluate_identity_map(&inputs(vec![special], vec![episode("d1", "1", "1")]));
        let blocked = outcome.blocked();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].reason, MergeBlockReason::UnmappedEpisode);
        assert_eq!(blocked[0].table, "episodes");
        assert_eq!(blocked[0].source_id, "s1");
    }

    #[test]
    fn absolute_numbering_resolves_when_the_season_pair_finds_nothing() {
        let mut source = episode("s1", "1", "13");
        source.absolute_number = Some("13".to_string());
        let mut destination = EpisodeIdentityFacts {
            id: "d1".to_string(),
            episode_type: EpisodeType::Standard,
            season_number: None,
            episode_number: None,
            absolute_number: Some("013".to_string()),
            collection_id: None,
        };
        destination.absolute_number = Some("13".to_string());
        let outcome = evaluate_identity_map(&inputs(vec![source], vec![destination]));
        assert_eq!(
            outcome.mapped().expect("absolute fallback maps").episode("s1"),
            Some("d1")
        );
    }

    #[test]
    fn two_destination_episodes_at_one_slot_are_ambiguous() {
        let outcome = evaluate_identity_map(&inputs(
            vec![episode("s1", "1", "1")],
            vec![episode("d1", "1", "1"), episode("d2", "01", "1")],
        ));
        assert_eq!(
            outcome.blocked()[0].reason,
            MergeBlockReason::AmbiguousDestinationEpisode
        );
    }

    #[test]
    fn two_source_episodes_at_one_slot_block_rather_than_collapse() {
        let outcome = evaluate_identity_map(&inputs(
            vec![episode("s1", "1", "1"), episode("s2", "1", "1")],
            vec![episode("d1", "1", "1")],
        ));
        let reasons: Vec<_> = outcome.blocked().iter().map(|r| r.reason).collect();
        assert_eq!(
            reasons,
            vec![
                MergeBlockReason::AmbiguousSourceEpisode,
                MergeBlockReason::AmbiguousSourceEpisode
            ]
        );
    }

    #[test]
    fn an_episode_with_no_numbers_at_all_is_unidentifiable() {
        let bare = EpisodeIdentityFacts {
            id: "s1".to_string(),
            episode_type: EpisodeType::Standard,
            season_number: None,
            episode_number: Some("not-a-number".to_string()),
            absolute_number: None,
            collection_id: None,
        };
        let outcome = evaluate_identity_map(&inputs(vec![bare], vec![episode("d1", "1", "1")]));
        assert_eq!(
            outcome.blocked()[0].reason,
            MergeBlockReason::UnidentifiableEpisode
        );
    }

    #[test]
    fn every_blocking_table_reports_its_own_unmapped_episode() {
        let mut input = inputs(
            vec![episode("s1", "1", "1"), episode("s2", "1", "2")],
            vec![episode("d1", "1", "1")],
        );
        for table in ["wanted_items", "subtitle_downloads", "domain_events"] {
            input.episode_references.insert(
                table.to_string(),
                BTreeSet::from(["s1".to_string(), "s2".to_string()]),
            );
        }
        let blocked = evaluate_identity_map(&input);
        let tables: Vec<_> = blocked
            .blocked()
            .iter()
            .map(|record| record.table.as_str())
            .collect();
        // `s1` maps, so only `s2` blocks — once for the map itself and once per
        // referencing table.
        assert_eq!(
            tables,
            vec![
                "domain_events",
                "episodes",
                "subtitle_downloads",
                "wanted_items"
            ]
        );
        assert!(
            blocked
                .blocked()
                .iter()
                .all(|record| record.source_id == "s2")
        );
    }

    #[test]
    fn a_reference_to_a_foreign_episode_is_its_own_reason() {
        let mut input = inputs(vec![episode("s1", "1", "1")], vec![episode("d1", "1", "1")]);
        input.episode_references.insert(
            "wanted_items".to_string(),
            BTreeSet::from(["not-ours".to_string()]),
        );
        assert_eq!(
            evaluate_identity_map(&input).blocked()[0].reason,
            MergeBlockReason::UnknownEpisodeReference
        );
    }

    #[test]
    fn a_table_outside_the_blocking_set_is_reported_not_evaluated() {
        let mut input = inputs(vec![episode("s1", "1", "1")], vec![episode("d1", "1", "1")]);
        input.episode_references.insert(
            "scope_indexer_coverage".to_string(),
            BTreeSet::from(["ghost".to_string()]),
        );
        let map = input;
        let outcome = evaluate_identity_map(&map);
        assert_eq!(
            outcome
                .mapped()
                .expect("a non-blocking table never blocks")
                .unevaluated_reference_tables,
            vec!["scope_indexer_coverage".to_string()]
        );
    }

    #[test]
    fn collections_map_on_type_and_index() {
        let mut input = inputs(vec![], vec![]);
        input.source_collections = vec![CollectionIdentityFacts {
            id: "sc1".to_string(),
            collection_type: CollectionType::Season,
            collection_index: "01".to_string(),
        }];
        input.destination_collections = vec![CollectionIdentityFacts {
            id: "dc1".to_string(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
        }];
        assert_eq!(
            evaluate_identity_map(&input)
                .mapped()
                .expect("the season maps")
                .collection("sc1"),
            Some("dc1")
        );
    }

    #[test]
    fn an_unmatched_collection_blocks() {
        let mut input = inputs(vec![], vec![]);
        input.source_collections = vec![CollectionIdentityFacts {
            id: "sc1".to_string(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
        }];
        assert_eq!(
            evaluate_identity_map(&input).blocked()[0].reason,
            MergeBlockReason::UnmappedCollection
        );
    }

    #[test]
    fn links_map_on_the_shared_movie_entity_and_flag_the_legacy_collision() {
        let mut input = inputs(vec![], vec![]);
        input.source_links = vec![SeriesMovieLinkIdentityFacts {
            id: "sl1".to_string(),
            movie_entity_id: "movie-1".to_string(),
            legacy_collection_id: Some("legacy-1".to_string()),
        }];
        input.destination_links = vec![SeriesMovieLinkIdentityFacts {
            id: "dl1".to_string(),
            movie_entity_id: "movie-1".to_string(),
            legacy_collection_id: Some("legacy-1".to_string()),
        }];
        let outcome = evaluate_identity_map(&input);
        let map = outcome.mapped().expect("the link maps");
        assert_eq!(map.series_movie_link("sl1"), Some("dl1"));
        assert_eq!(map.legacy_collection_ids_to_clear, vec!["sl1".to_string()]);
    }

    #[test]
    fn a_resumable_operation_holding_the_source_hard_blocks_oq7() {
        let mut input = inputs(vec![episode("s1", "1", "1")], vec![episode("d1", "1", "1")]);
        input.resumable_operations_holding_source = vec!["op-9".to_string()];
        let blocked = evaluate_identity_map(&input);
        assert_eq!(
            blocked.blocked()[0].reason,
            MergeBlockReason::ResumableOperationHoldsSource
        );
        assert_eq!(blocked.blocked()[0].table, "location_operations");
    }

    #[test]
    fn an_unconsumed_manual_import_selection_blocks() {
        let mut input = inputs(vec![episode("s1", "1", "1")], vec![episode("d1", "1", "1")]);
        input.unconsumed_manual_import_selections = vec!["sel-1".to_string()];
        assert_eq!(
            evaluate_identity_map(&input).blocked()[0].reason,
            MergeBlockReason::ActiveManualImportSelection
        );
    }

    #[test]
    fn a_blocked_record_names_its_table_and_episode_for_the_checkpoint() {
        let record = MergeBlockedRecord {
            table: "wanted_items".to_string(),
            reason: MergeBlockReason::UnmappedEpisode,
            source_id: "episode-7".to_string(),
            detail: "no destination episode carries standard S02E03".to_string(),
        };
        assert_eq!(
            record.summary_line(),
            "wanted_items (unmapped_episode): episode-7 — no destination episode carries standard \
             S02E03"
        );
    }
}
