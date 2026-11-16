//! Scoped lifecycle identity and current catalog membership. Never infer a
//! missing child identity from its parent title.
use crate::types::{EpisodeScopedMediaFile, TitleMediaFile};
use crate::{AppError, AppResult, AppUseCase};
use chrono::{DateTime, NaiveDate, Utc};
use scryer_domain::{
    Collection, CollectionType, Episode, EpisodeType, LifecycleRuleExclusion,
    LifecycleRuleSubjectKind as Scope, Title,
};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// The episode inventory for one non-movie title. The loader reads it once for
/// a bounded title page; callers may reuse the `Arc` across rule scopes but
/// must discard it at the page boundary so previews and execution rechecks
/// always start from a fresh catalog/file snapshot.
#[derive(Debug)]
pub(crate) struct LifecycleTitleEpisodeInventory {
    title_id: String,
    collections: Vec<Collection>,
    episodes: Vec<Episode>,
    scoped_files: Vec<EpisodeScopedMediaFile>,
    file_availability: HashMap<String, EpisodeFileAvailability>,
    episode_coverage: HashMap<String, EpisodeFileCoverage>,
    retention_ranks: Mutex<Option<(DateTime<Utc>, Arc<LifecycleRetentionRanks>)>>,
}

#[derive(Clone, Debug, Default)]
struct EpisodeFileCoverage {
    has_present_file: bool,
    has_missing_file: bool,
    has_unreadable_file: bool,
    ambiguous: bool,
}

impl EpisodeFileCoverage {
    fn downloaded(&self) -> bool {
        matches!(
            (
                self.has_present_file,
                self.has_missing_file,
                self.has_unreadable_file
            ),
            (true, _, false)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EpisodeFileAvailability {
    Present,
    Missing,
    Unreadable,
}

/// Parent-show retention ranks computed from one current inventory.
#[derive(Clone, Debug, Default)]
pub(crate) struct LifecycleRetentionRanks {
    pub episode_positions: HashMap<String, i64>,
    pub season_positions: HashMap<String, i64>,
}

impl LifecycleTitleEpisodeInventory {
    fn downloaded_episode_count(&self, episodes: &[Episode]) -> (i64, bool) {
        let mut count = 0;
        let mut unresolved = false;
        for episode in episodes {
            let coverage = self.episode_coverage.get(&episode.id);
            unresolved |= coverage.is_some_and(|coverage| coverage.has_unreadable_file);
            count += i64::from(coverage.is_some_and(EpisodeFileCoverage::downloaded));
        }
        (count, unresolved)
    }

    /// Rank only files whose catalogue ownership, episode association, and
    /// physical presence are all proven. Ineligible episodes deliberately do
    /// not receive a position: the fact builder turns that into unknown, not
    /// an absent rank that a negated policy check could treat as permission.
    pub(crate) fn retention_ranks_at(
        &self,
        evaluation_time: DateTime<Utc>,
    ) -> Arc<LifecycleRetentionRanks> {
        {
            let cache = self
                .retention_ranks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some((cached_at, ranks)) = cache.as_ref()
                && cached_at == &evaluation_time
            {
                return Arc::clone(ranks);
            }
        }
        let ranks = Arc::new(self.calculate_retention_ranks(evaluation_time, None));
        let mut cache = self
            .retention_ranks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached_at, ranks)) = cache.as_ref()
            && cached_at == &evaluation_time
        {
            return Arc::clone(ranks);
        }
        *cache = Some((evaluation_time, Arc::clone(&ranks)));
        ranks
    }

    /// The episodes a tracked file is linked to, or `None` for a file this
    /// inventory does not hold. File rows read for a scope carry no episode
    /// of their own; their links live here.
    pub(crate) fn file_episode_ids(&self, file_id: &str) -> Option<&[String]> {
        self.scoped_files
            .iter()
            .find(|scoped| scoped.media_file.id == file_id)
            .map(|scoped| scoped.episode_ids.as_slice())
    }

    /// File ids are captured in the durable deletion journal before unlinking.
    /// Only a currently present, unambiguous file that contributed to a rank
    /// may restore its episode when that same journal later resumes.
    pub(crate) fn retention_episode_ids_for_file_ids(
        &self,
        file_ids: &HashSet<String>,
    ) -> BTreeSet<String> {
        self.scoped_files
            .iter()
            .filter(|scoped| {
                file_ids.contains(&scoped.media_file.id)
                    && self.file_availability.get(&scoped.media_file.id)
                        == Some(&EpisodeFileAvailability::Present)
                    && scoped.media_file.title_id == self.title_id
                    && scoped.media_file.series_movie_link_ids.is_empty()
                    && scoped.episode_ids.len() == 1
            })
            .filter_map(|scoped| scoped.episode_ids.first())
            .filter(|episode_id| {
                self.episodes
                    .iter()
                    .any(|episode| episode.id.as_str() == episode_id.as_str())
                    && self.episode_has_retention_coverage(episode_id, None)
            })
            .cloned()
            .collect()
    }

    /// Recompute ranks from live inventory while restoring only the completed
    /// coverage recorded by this candidate's own deletion journal. This keeps
    /// imports and other candidates' deletions visible during a partial run.
    pub(crate) fn retention_ranks_with_completed_episode_coverage(
        &self,
        evaluation_time: DateTime<Utc>,
        completed_episode_ids: &BTreeSet<String>,
    ) -> Arc<LifecycleRetentionRanks> {
        if completed_episode_ids.is_empty() {
            return self.retention_ranks_at(evaluation_time);
        }
        Arc::new(self.calculate_retention_ranks(evaluation_time, Some(completed_episode_ids)))
    }

    fn calculate_retention_ranks(
        &self,
        evaluation_time: DateTime<Utc>,
        completed_episode_ids: Option<&BTreeSet<String>>,
    ) -> LifecycleRetentionRanks {
        let special_collections: HashSet<&str> = self
            .collections
            .iter()
            .filter(|collection| {
                collection.collection_type == CollectionType::Specials
                    || is_special_season_number(Some(&collection.collection_index))
            })
            .map(|collection| collection.id.as_str())
            .collect();
        let mut eligible: Vec<&Episode> = self
            .episodes
            .iter()
            .filter(|episode| {
                episode.episode_type != EpisodeType::Special
                    && !is_special_season_number(episode.season_number.as_deref())
                    && !episode
                        .collection_id
                        .as_deref()
                        .is_some_and(|id| special_collections.contains(id))
                    && self.episode_has_retention_coverage(&episode.id, completed_episode_ids)
                    && air_date_is_aired(episode.air_date.as_deref(), evaluation_time).is_some()
            })
            .collect();
        eligible.sort_by(|left, right| compare_episode_rank(left, right, evaluation_time));

        let episode_positions = eligible
            .iter()
            .enumerate()
            .map(|(index, episode)| (episode.id.clone(), index as i64 + 1))
            .collect();

        let mut newest_by_season: HashMap<&str, &Episode> = HashMap::new();
        for episode in &eligible {
            let Some(collection_id) = episode.collection_id.as_deref() else {
                continue;
            };
            let Some(collection) = self
                .collections
                .iter()
                .find(|collection| collection.id == collection_id)
            else {
                continue;
            };
            if collection.collection_type != CollectionType::Season
                || is_special_season_number(Some(&collection.collection_index))
            {
                continue;
            }
            newest_by_season
                .entry(collection_id)
                .and_modify(|current| {
                    if compare_episode_rank(episode, current, evaluation_time).is_lt() {
                        *current = episode;
                    }
                })
                .or_insert(episode);
        }
        let mut seasons: Vec<(&str, &Episode)> = newest_by_season.into_iter().collect();
        seasons.sort_by(|(left_id, left), (right_id, right)| {
            compare_episode_rank(left, right, evaluation_time).then_with(|| left_id.cmp(right_id))
        });
        let season_positions = seasons
            .into_iter()
            .enumerate()
            .map(|(index, (collection_id, _))| (collection_id.to_string(), index as i64 + 1))
            .collect();

        LifecycleRetentionRanks {
            episode_positions,
            season_positions,
        }
    }

    fn episode_has_retention_coverage(
        &self,
        episode_id: &str,
        completed_episode_ids: Option<&BTreeSet<String>>,
    ) -> bool {
        let restored = completed_episode_ids.is_some_and(|ids| ids.contains(episode_id));
        match self.episode_coverage.get(episode_id) {
            Some(coverage) => {
                !coverage.ambiguous
                    && !coverage.has_unreadable_file
                    && (coverage.has_present_file || restored)
            }
            None => restored,
        }
    }
}

fn air_date_is_aired(raw: Option<&str>, evaluation_time: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let instant = DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|instant| instant.with_timezone(&Utc))
        .or_else(|| {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|instant| instant.and_utc())
        })?;
    (instant <= evaluation_time).then_some(instant)
}

fn compare_episode_rank(
    left: &Episode,
    right: &Episode,
    evaluation_time: DateTime<Utc>,
) -> std::cmp::Ordering {
    let left_date = air_date_is_aired(left.air_date.as_deref(), evaluation_time)
        .expect("only eligible episodes are ranked");
    let right_date = air_date_is_aired(right.air_date.as_deref(), evaluation_time)
        .expect("only eligible episodes are ranked");
    right_date
        .cmp(&left_date)
        .then_with(|| {
            compare_number_desc(
                left.season_number.as_deref(),
                right.season_number.as_deref(),
            )
        })
        .then_with(|| {
            compare_number_desc(
                left.episode_number.as_deref(),
                right.episode_number.as_deref(),
            )
        })
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_number_desc(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
    match (
        left.and_then(|value| value.parse::<i64>().ok()),
        right.and_then(|value| value.parse::<i64>().ok()),
    ) {
        (Some(left), Some(right)) => right.cmp(&left),
        _ => right.cmp(&left),
    }
}

fn is_special_season_number(value: Option<&str>) -> bool {
    value
        .map(str::trim)
        .and_then(|value| value.parse::<i64>().ok())
        == Some(0)
}

#[derive(Clone, Debug)]
pub(crate) struct LifecycleSubject {
    pub kind: Scope,
    pub id: String,
    pub collection: Option<Collection>,
    pub episodes: Vec<Episode>,
    pub files: Vec<TitleMediaFile>,
    pub episode_file_count: i64,
    pub episode_file_count_unresolved: bool,
    pub ambiguous_files: bool,
    pub inventory: Option<Arc<LifecycleTitleEpisodeInventory>>,
    pub monitored: bool,
    pub label: String,
}

impl LifecycleSubject {
    pub fn excluded_by(&self, title_id: &str, exclusions: &[LifecycleRuleExclusion]) -> bool {
        exclusions.iter().any(|exclusion| {
            exclusion.title_id == title_id
                && match exclusion.subject_kind {
                    Scope::Title => exclusion.subject_id == title_id,
                    Scope::Season => self
                        .collection
                        .as_ref()
                        .is_some_and(|collection| collection.id == exclusion.subject_id),
                    Scope::Episode => {
                        self.kind == Scope::Episode && self.id == exclusion.subject_id
                    }
                }
        })
    }
}

impl AppUseCase {
    pub(crate) async fn lifecycle_subjects_for_title(
        &self,
        kind: Scope,
        title: &Title,
        title_files: &[TitleMediaFile],
    ) -> AppResult<Vec<LifecycleSubject>> {
        if title.facet == scryer_domain::MediaFacet::Movie {
            return Ok((kind == Scope::Title)
                .then(|| LifecycleSubject {
                    kind,
                    id: title.id.clone(),
                    collection: None,
                    episodes: vec![],
                    files: title_files.to_vec(),
                    episode_file_count: 0,
                    episode_file_count_unresolved: false,
                    ambiguous_files: title_files.iter().any(|file| {
                        !file.series_movie_link_ids.is_empty() || file.title_id != title.id
                    }),
                    inventory: None,
                    monitored: title.monitored,
                    label: title.name.clone(),
                })
                .into_iter()
                .collect());
        }

        let inventory = self.lifecycle_load_title_episode_inventory(title).await?;
        self.lifecycle_subjects_from_episode_inventory(kind, title, title_files, &inventory)
    }

    /// Load the title's complete episode/file inventory once. The returned
    /// `Arc` is immutable and title-bound, so a bounded evaluation-page cache
    /// can share it across scope/rule projections without cross-title leakage.
    pub(crate) async fn lifecycle_load_title_episode_inventory(
        &self,
        title: &Title,
    ) -> AppResult<Arc<LifecycleTitleEpisodeInventory>> {
        self.load_title_episode_inventory(title, FilePresence::Probe(None))
            .await
    }

    /// [`Self::lifecycle_load_title_episode_inventory`] for an evaluation
    /// pass: a file whose presence this pass already probed for another rule
    /// is answered from `presence` instead of probing the share again.
    pub(crate) async fn lifecycle_load_title_episode_inventory_for_pass(
        &self,
        title: &Title,
        presence: &mut LifecyclePassFilePresence,
    ) -> AppResult<Arc<LifecycleTitleEpisodeInventory>> {
        self.load_title_episode_inventory(title, FilePresence::Probe(Some(presence)))
            .await
    }

    /// The episode inventories of a page of titles, read together: one read
    /// each of collections, episodes and episode files for every show in
    /// `titles`, then each inventory built exactly as
    /// [`Self::lifecycle_load_title_episode_inventory`] builds it, in title
    /// order. Movies get none.
    ///
    /// The map is a head start, never the answer. A show it lacks is loaded
    /// through the per-title path, which then succeeds or fails as it always
    /// did: when a batched read fails nothing is returned, and when one
    /// title's inventory cannot be built, it and every later title are left
    /// out, so no file is probed that the per-title path would not reach.
    /// `presence` is the pass memo; `None` probes each file the way the
    /// per-title loader does without one.
    pub(crate) async fn lifecycle_load_title_episode_inventories(
        &self,
        titles: &[Title],
        mut presence: Option<&mut LifecyclePassFilePresence>,
    ) -> HashMap<String, Arc<LifecycleTitleEpisodeInventory>> {
        let shows: Vec<&Title> = titles
            .iter()
            .filter(|title| title.facet != scryer_domain::MediaFacet::Movie)
            .collect();
        let mut inventories = HashMap::new();
        if shows.is_empty() {
            return inventories;
        }
        let show_ids: Vec<String> = shows.iter().map(|title| title.id.clone()).collect();
        let reads = async {
            let collections = self
                .services
                .catalog
                .shows
                .list_collections_for_titles(&show_ids)
                .await?;
            let mut episodes: HashMap<String, Vec<Episode>> = HashMap::new();
            for episode in self
                .services
                .catalog
                .shows
                .list_episodes_for_titles(&show_ids)
                .await?
            {
                episodes
                    .entry(episode.title_id.clone())
                    .or_default()
                    .push(episode);
            }
            let requests: Vec<(String, Vec<String>)> = show_ids
                .iter()
                .filter_map(|title_id| {
                    let ids: Vec<String> = episodes
                        .get(title_id)?
                        .iter()
                        .map(|episode| episode.id.clone())
                        .collect();
                    Some((title_id.clone(), ids))
                })
                .collect();
            let files: HashMap<String, Vec<EpisodeScopedMediaFile>> = if requests.is_empty() {
                HashMap::new()
            } else {
                self.services
                    .library
                    .media_files
                    .list_live_media_files_for_titles_episode_ids(&requests)
                    .await?
                    .into_iter()
                    .collect()
            };
            AppResult::Ok((collections, episodes, files))
        };
        let (mut collections, mut episodes, mut files) = match reads.await {
            Ok(reads) => reads,
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "batched episode inventory read failed; titles load one at a time"
                );
                return inventories;
            }
        };
        for title in shows {
            let episodes = episodes.remove(&title.id).unwrap_or_default();
            let mut scoped = files.remove(&title.id).unwrap_or_default();
            {
                let read_of = episode_reads(&episodes);
                for file in &mut scoped {
                    per_title_read_role(file, &read_of);
                }
            }
            let presence = match presence.as_deref_mut() {
                Some(memo) => FilePresence::Probe(Some(memo)),
                None => FilePresence::Probe(None),
            };
            match self
                .build_title_episode_inventory(
                    title,
                    collections.remove(&title.id).unwrap_or_default(),
                    episodes,
                    scoped,
                    presence,
                )
                .await
            {
                Ok(inventory) => {
                    inventories.insert(title.id.clone(), inventory);
                }
                Err(_) => break,
            }
        }
        inventories
    }

    /// [`FilePresence::Skip`] only for a candidate listing, through
    /// [`LifecycleListingInventories`], which never hands the inventory on.
    async fn load_title_episode_inventory(
        &self,
        title: &Title,
        presence: FilePresence<'_>,
    ) -> AppResult<Arc<LifecycleTitleEpisodeInventory>> {
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;
        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;
        // Ownership is proved before any file is read.
        episodes_are_owned(title, &collections, &episodes)?;
        // Bound the relation query. Keep complete coverage returned for each
        // file, including associations outside the selected episode batch.
        let mut scoped_files = Vec::new();
        for batch in episodes.chunks(INVENTORY_EPISODE_READ_CHUNK) {
            let ids: Vec<String> = batch.iter().map(|episode| episode.id.clone()).collect();
            scoped_files.extend(
                self.services
                    .library
                    .media_files
                    .list_live_media_files_for_episode_ids(&title.id, &ids)
                    .await?,
            );
        }
        self.build_title_episode_inventory(title, collections, episodes, scoped_files, presence)
            .await
    }

    /// One title's inventory from rows already read. `scoped_files` may hold
    /// a file more than once; the last copy is the one kept.
    async fn build_title_episode_inventory(
        &self,
        title: &Title,
        collections: Vec<Collection>,
        episodes: Vec<Episode>,
        scoped_files: Vec<EpisodeScopedMediaFile>,
        mut presence: FilePresence<'_>,
    ) -> AppResult<Arc<LifecycleTitleEpisodeInventory>> {
        episodes_are_owned(title, &collections, &episodes)?;
        let mut deduplicated = HashMap::new();
        for file in scoped_files {
            deduplicated.insert(file.media_file.id.clone(), file);
        }
        let mut scoped_files: Vec<_> = deduplicated.into_values().collect();
        scoped_files.sort_by(|left, right| left.media_file.id.cmp(&right.media_file.id));
        let episode_ids: HashSet<&str> =
            episodes.iter().map(|episode| episode.id.as_str()).collect();
        let mut episode_coverage: HashMap<String, EpisodeFileCoverage> = HashMap::new();
        let mut file_availabilities = HashMap::new();
        for scoped in &scoped_files {
            let availability = match &mut presence {
                FilePresence::Skip => None,
                FilePresence::Probe(None) => {
                    Some(file_availability(&scoped.media_file.file_path).await)
                }
                FilePresence::Probe(Some(memo)) => {
                    let path = &scoped.media_file.file_path;
                    Some(match memo.by_path.get(path) {
                        Some(availability) => *availability,
                        None => {
                            let availability = file_availability(path).await;
                            memo.by_path.insert(path.clone(), availability);
                            availability
                        }
                    })
                }
            };
            if let Some(availability) = availability {
                file_availabilities.insert(scoped.media_file.id.clone(), availability);
            }
            let associated_ids: BTreeSet<&str> = scoped
                .episode_ids
                .iter()
                .map(String::as_str)
                .filter(|id| episode_ids.contains(id))
                .collect();
            let ambiguous = scoped.media_file.title_id != title.id
                || !scoped.media_file.series_movie_link_ids.is_empty()
                || scoped.episode_ids.len() != 1
                || scoped
                    .episode_ids
                    .iter()
                    .any(|id| !episode_ids.contains(id.as_str()));
            for episode_id in associated_ids {
                let coverage = episode_coverage.entry(episode_id.to_string()).or_default();
                coverage.ambiguous |= ambiguous;
                match availability {
                    Some(EpisodeFileAvailability::Present) => coverage.has_present_file = true,
                    Some(EpisodeFileAvailability::Missing) => coverage.has_missing_file = true,
                    Some(EpisodeFileAvailability::Unreadable) => {
                        coverage.has_unreadable_file = true
                    }
                    None => {}
                }
            }
        }

        Ok(Arc::new(LifecycleTitleEpisodeInventory {
            title_id: title.id.clone(),
            collections,
            episodes,
            scoped_files,
            file_availability: file_availabilities,
            episode_coverage,
            retention_ranks: Mutex::default(),
        }))
    }

    /// Project one title-bound inventory into the requested scope without any
    /// repository or filesystem reads. Evaluation caches this only within one
    /// bounded title page; execution calls the fresh-loading wrapper above.
    pub(crate) fn lifecycle_subjects_from_episode_inventory(
        &self,
        kind: Scope,
        title: &Title,
        title_files: &[TitleMediaFile],
        inventory: &Arc<LifecycleTitleEpisodeInventory>,
    ) -> AppResult<Vec<LifecycleSubject>> {
        if inventory.title_id != title.id {
            return Err(AppError::Repository(
                "lifecycle episode inventory title mismatch".into(),
            ));
        }
        if title.facet == scryer_domain::MediaFacet::Movie {
            return Err(AppError::validation_refused(
                crate::error_reasons::lifecycle::MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY,
                "movie titles do not have an episode inventory",
            ));
        }
        if kind == Scope::Title {
            let (episode_file_count, episode_file_count_unresolved) =
                inventory.downloaded_episode_count(&inventory.episodes);
            return Ok(vec![LifecycleSubject {
                kind,
                id: title.id.clone(),
                collection: None,
                episodes: inventory.episodes.clone(),
                files: title_files.to_vec(),
                episode_file_count,
                episode_file_count_unresolved,
                ambiguous_files: title_files.iter().any(|file| {
                    !file.series_movie_link_ids.is_empty() || file.title_id != title.id
                }),
                inventory: Some(Arc::clone(inventory)),
                monitored: title.monitored,
                label: title.name.clone(),
            }]);
        }

        let mut subjects = Vec::new();
        if kind == Scope::Season {
            for collection in inventory.collections.iter().filter(|collection| {
                matches!(
                    collection.collection_type,
                    CollectionType::Season | CollectionType::Specials
                )
            }) {
                subjects.push(LifecycleSubject {
                    kind,
                    id: collection.id.clone(),
                    collection: Some(collection.clone()),
                    episodes: inventory
                        .episodes
                        .iter()
                        .filter(|episode| episode.collection_id.as_deref() == Some(&collection.id))
                        .cloned()
                        .collect(),
                    files: vec![],
                    episode_file_count: 0,
                    episode_file_count_unresolved: false,
                    ambiguous_files: false,
                    inventory: Some(Arc::clone(inventory)),
                    monitored: collection.monitored,
                    label: format!("Season {}", collection.collection_index),
                });
            }
        } else {
            for episode in &inventory.episodes {
                let collection = inventory
                    .collections
                    .iter()
                    .find(|collection| Some(&collection.id) == episode.collection_id.as_ref())
                    .cloned();
                subjects.push(LifecycleSubject {
                    kind,
                    id: episode.id.clone(),
                    collection,
                    episodes: vec![episode.clone()],
                    files: vec![],
                    episode_file_count: 0,
                    episode_file_count_unresolved: false,
                    ambiguous_files: false,
                    inventory: Some(Arc::clone(inventory)),
                    monitored: episode.monitored,
                    label: format!(
                        "S{}E{}{}",
                        episode.season_number.as_deref().unwrap_or("?"),
                        episode.episode_number.as_deref().unwrap_or("?"),
                        episode
                            .title
                            .as_ref()
                            .map(|name| format!(" · {name}"))
                            .unwrap_or_default()
                    ),
                });
            }
        }
        for subject in &mut subjects {
            let ids: HashSet<&str> = subject
                .episodes
                .iter()
                .map(|episode| episode.id.as_str())
                .collect();
            for scoped in &inventory.scoped_files {
                if !scoped
                    .episode_ids
                    .iter()
                    .any(|id| ids.contains(id.as_str()))
                {
                    continue;
                }
                subject.ambiguous_files |= scoped.media_file.title_id != title.id
                    || scoped
                        .episode_ids
                        .iter()
                        .any(|id| !ids.contains(id.as_str()))
                    || !scoped.media_file.series_movie_link_ids.is_empty();
                subject.files.push(scoped.media_file.clone());
            }
            let (episode_file_count, episode_file_count_unresolved) =
                inventory.downloaded_episode_count(&subject.episodes);
            subject.episode_file_count = episode_file_count;
            subject.episode_file_count_unresolved = episode_file_count_unresolved;
            subject.files.sort_by(|a, b| a.id.cmp(&b.id));
        }
        Ok(subjects)
    }

    pub(crate) async fn lifecycle_resolve_subject(
        &self,
        kind: &str,
        subject_id: &str,
        title: &Title,
        files: &[TitleMediaFile],
    ) -> AppResult<LifecycleSubject> {
        let kind = subject_coordinates_kind(kind, subject_id)?;
        self.lifecycle_subjects_for_title(kind, title, files)
            .await?
            .into_iter()
            .find(|subject| subject.id == subject_id)
            .ok_or_else(subject_no_longer_belongs)
    }

    /// [`Self::lifecycle_subjects_for_title`] for a candidate listing: each
    /// title's inventory is read once per listing, without statting files.
    pub(crate) async fn lifecycle_listing_subjects(
        &self,
        kind: Scope,
        title: &Title,
        title_files: &[TitleMediaFile],
        inventories: &mut LifecycleListingInventories,
    ) -> AppResult<Vec<LifecycleListingSubject>> {
        let subjects = if title.facet == scryer_domain::MediaFacet::Movie {
            self.lifecycle_subjects_for_title(kind, title, title_files)
                .await?
        } else {
            let inventory = match inventories.by_title.get(&title.id) {
                Some(inventory) => Arc::clone(inventory),
                None => {
                    let inventory = self
                        .load_title_episode_inventory(title, FilePresence::Skip)
                        .await?;
                    inventories
                        .by_title
                        .insert(title.id.clone(), Arc::clone(&inventory));
                    inventory
                }
            };
            self.lifecycle_subjects_from_episode_inventory(kind, title, title_files, &inventory)?
        };
        Ok(subjects
            .into_iter()
            .map(LifecycleListingSubject::from)
            .collect())
    }
}

fn subject_coordinates_kind(kind: &str, subject_id: &str) -> AppResult<Scope> {
    let kind = Scope::parse_storage(kind).ok_or_else(|| {
        AppError::validation_refused(
            crate::error_reasons::lifecycle::SUBJECT_KIND_INVALID,
            "invalid lifecycle subject kind",
        )
    })?;
    if subject_id.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::lifecycle::SUBJECT_ID_REQUIRED,
            "lifecycle subject ID is required",
        ));
    }
    Ok(kind)
}

fn subject_no_longer_belongs() -> AppError {
    AppError::not_found_reason(
        crate::error_reasons::lifecycle::SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND,
        "lifecycle subject no longer belongs to this title",
    )
}

/// The display fields of one subject in a candidate listing. It carries no
/// inventory and no downloaded-episode counts, so nothing built from it can
/// become a rule fact or an action input.
#[derive(Debug)]
pub(crate) struct LifecycleListingSubject {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) files: Vec<TitleMediaFile>,
}

impl From<LifecycleSubject> for LifecycleListingSubject {
    fn from(subject: LifecycleSubject) -> Self {
        Self {
            id: subject.id,
            label: subject.label,
            files: subject.files,
        }
    }
}

/// Episode inventories for one candidate listing, read once per title. They
/// are read without statting files, since no listing field depends on file
/// presence, and they stay private to this type so evaluation and execution
/// can never receive one.
#[derive(Default)]
pub(crate) struct LifecycleListingInventories {
    by_title: HashMap<String, Arc<LifecycleTitleEpisodeInventory>>,
}

/// Episodes per inventory read of one title; the per-title loader reads a
/// title's files this many episodes at a time.
const INVENTORY_EPISODE_READ_CHUNK: usize = 100;

/// Every episode and collection belongs to the title, and every episode's
/// collection is one of the title's.
fn episodes_are_owned(
    title: &Title,
    collections: &[Collection],
    episodes: &[Episode],
) -> AppResult<()> {
    if episodes.iter().any(|episode| episode.title_id != title.id)
        || collections
            .iter()
            .any(|collection| collection.title_id != title.id)
    {
        return Err(AppError::Repository(
            "lifecycle subject ownership mismatch".into(),
        ));
    }
    let collection_ids: HashSet<&str> = collections
        .iter()
        .map(|collection| collection.id.as_str())
        .collect();
    if episodes.iter().any(|episode| {
        episode
            .collection_id
            .as_ref()
            .is_some_and(|id| !collection_ids.contains(id.as_str()))
    }) {
        return Err(AppError::Repository(
            "lifecycle episode collection ownership mismatch".into(),
        ));
    }
    Ok(())
}

/// The role the per-title loader leaves on a file: it reads a title's files
/// [`INVENTORY_EPISODE_READ_CHUNK`] episodes at a time, marks a file primary
/// when it is primary for an episode of that read, and keeps the last read's
/// copy. The batched read marks it over all of the title's episodes, so a file
/// spanning two reads is set back to what the last of them said.
fn per_title_read_role(file: &mut EpisodeScopedMediaFile, read_of: &HashMap<&str, usize>) {
    let Some(last_read) = file
        .episode_ids
        .iter()
        .filter_map(|id| read_of.get(id.as_str()))
        .max()
        .copied()
    else {
        return;
    };
    file.media_file.role = if file
        .primary_episode_ids
        .iter()
        .any(|id| read_of.get(id.as_str()) == Some(&last_read))
    {
        crate::MediaFileRole::Primary
    } else {
        crate::MediaFileRole::Additional
    };
}

/// Which of the per-title loader's reads each episode of a title falls in.
fn episode_reads(episodes: &[Episode]) -> HashMap<&str, usize> {
    episodes
        .iter()
        .enumerate()
        .map(|(index, episode)| (episode.id.as_str(), index / INVENTORY_EPISODE_READ_CHUNK))
        .collect()
}

/// Whether an inventory load probes its files' presence on disk.
enum FilePresence<'a> {
    /// A candidate listing: no listing field depends on presence.
    Skip,
    /// Probe each file, answering from the pass memo when one is given.
    Probe(Option<&'a mut LifecyclePassFilePresence>),
}

/// File presence probed during one evaluation pass, by catalog path. Every
/// rule of the pass sees the presence sampled the first time any rule loaded
/// that file, the way rules already shared a cached title inventory. It lives
/// only for the pass; previews and execution always probe afresh.
#[derive(Default)]
pub(crate) struct LifecyclePassFilePresence {
    by_path: HashMap<String, EpisodeFileAvailability>,
}

#[cfg(test)]
impl LifecyclePassFilePresence {
    pub(crate) fn probed_paths(&self) -> usize {
        self.by_path.len()
    }
}

#[cfg(test)]
impl LifecycleTitleEpisodeInventory {
    /// Everything the inventory holds, in a fixed order, so two loads can be
    /// compared field by field.
    pub(crate) fn comparable(&self) -> String {
        let files: Vec<String> = self
            .scoped_files
            .iter()
            .map(|scoped| format!("{scoped:?}"))
            .collect();
        let mut availability: Vec<_> = self.file_availability.iter().collect();
        availability.sort_by(|left, right| left.0.cmp(right.0));
        let mut coverage: Vec<_> = self.episode_coverage.iter().collect();
        coverage.sort_by(|left, right| left.0.cmp(right.0));
        format!(
            "title={}\ncollections={:?}\nepisodes={:?}\nfiles={files:?}\navailability={availability:?}\ncoverage={coverage:?}",
            self.title_id, self.collections, self.episodes
        )
    }
}

async fn file_availability(path: &str) -> EpisodeFileAvailability {
    if path.trim().is_empty() {
        return EpisodeFileAvailability::Missing;
    }
    let reading = crate::disk::Disk::process().follow(path).await;
    match reading.exists() {
        Some(true) if reading.is(crate::disk::EntryKind::File) => EpisodeFileAvailability::Present,
        Some(_) => EpisodeFileAvailability::Missing,
        None => EpisodeFileAvailability::Unreadable,
    }
}

impl AppUseCase {
    /// At most one subject load per (scope, title) for a bounded results page.
    pub(crate) async fn lifecycle_subject_summaries(
        &self,
        keys: impl Iterator<Item = (String, String)>,
    ) -> AppResult<std::collections::HashMap<(String, String), (String, i64, i64)>> {
        self.subject_summaries(keys, None).await
    }

    /// [`Self::lifecycle_subject_summaries`] for a candidate listing, which
    /// shares the listing's inventories.
    pub(crate) async fn lifecycle_listing_subject_summaries(
        &self,
        keys: impl Iterator<Item = (String, String)>,
        inventories: &mut LifecycleListingInventories,
    ) -> AppResult<std::collections::HashMap<(String, String), (String, i64, i64)>> {
        self.subject_summaries(keys, Some(inventories)).await
    }

    async fn subject_summaries(
        &self,
        keys: impl Iterator<Item = (String, String)>,
        mut inventories: Option<&mut LifecycleListingInventories>,
    ) -> AppResult<std::collections::HashMap<(String, String), (String, i64, i64)>> {
        let mut summaries = std::collections::HashMap::new();
        let mut keys: Vec<_> = keys.collect();
        keys.sort();
        keys.dedup();
        for (kind, title_id) in keys {
            let Some(kind) = Scope::parse_storage(&kind) else {
                continue;
            };
            let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
                continue;
            };
            let files = self
                .services
                .library
                .media_files
                .list_media_files_for_title(&title_id)
                .await?;
            let subjects = match inventories.as_deref_mut() {
                Some(inventories) => {
                    self.lifecycle_listing_subjects(kind, &title, &files, inventories)
                        .await?
                }
                None => self
                    .lifecycle_subjects_for_title(kind, &title, &files)
                    .await?
                    .into_iter()
                    .map(LifecycleListingSubject::from)
                    .collect(),
            };
            for subject in subjects {
                summaries.insert(
                    (kind.as_storage_str().to_string(), subject.id),
                    (
                        subject.label,
                        subject.files.len() as i64,
                        subject.files.iter().map(|file| file.size_bytes).sum(),
                    ),
                );
            }
        }
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A batched file row carries the role over all of a title's episodes;
    /// the per-title loader leaves the role its last 100-episode read gave.
    #[test]
    fn a_file_spanning_two_reads_keeps_the_role_of_the_last_one() {
        let episodes: Vec<Episode> = (0..150)
            .map(|index| {
                episode(
                    &format!("episode-{index:03}"),
                    "season-1",
                    "1",
                    &index.to_string(),
                    None,
                    EpisodeType::Standard,
                )
            })
            .collect();
        let read_of = episode_reads(&episodes);
        let scoped = |links: &[(usize, bool)]| EpisodeScopedMediaFile {
            media_file: TitleMediaFile {
                id: "file-fixture".to_string(),
                role: crate::MediaFileRole::Primary,
                ..Default::default()
            },
            title_role: crate::MediaFileRole::Primary,
            episode_ids: links
                .iter()
                .map(|(index, _)| format!("episode-{index:03}"))
                .collect(),
            primary_episode_ids: links
                .iter()
                .filter(|(_, primary)| *primary)
                .map(|(index, _)| format!("episode-{index:03}"))
                .collect(),
        };
        let role = |links: &[(usize, bool)]| {
            let mut file = scoped(links);
            per_title_read_role(&mut file, &read_of);
            file.media_file.role
        };
        // Primary in the first read only: the second read says additional.
        assert_eq!(
            role(&[(99, true), (100, false)]),
            crate::MediaFileRole::Additional
        );
        assert_eq!(
            role(&[(99, false), (100, true)]),
            crate::MediaFileRole::Primary
        );
        assert_eq!(
            role(&[(3, true), (4, false)]),
            crate::MediaFileRole::Primary
        );
        assert_eq!(
            role(&[(3, false), (4, false)]),
            crate::MediaFileRole::Additional
        );
        // A file linked to none of the title's episodes keeps its role.
        assert_eq!(role(&[]), crate::MediaFileRole::Primary);
    }

    fn collection(id: &str, index: &str, collection_type: CollectionType) -> Collection {
        Collection {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            collection_type,
            collection_index: index.to_string(),
            label: None,
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn episode(
        id: &str,
        collection_id: &str,
        season_number: &str,
        episode_number: &str,
        air_date: Option<&str>,
        episode_type: EpisodeType,
    ) -> Episode {
        Episode {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            collection_id: Some(collection_id.to_string()),
            episode_type,
            episode_number: Some(episode_number.to_string()),
            season_number: Some(season_number.to_string()),
            episode_label: None,
            title: None,
            air_date: air_date.map(str::to_string),
            duration_seconds: None,
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            contiguous_absolute_number: None,
            overview: None,
            tvdb_id: None,
            tmdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        }
    }

    fn coverage(
        present: bool,
        missing: bool,
        unreadable: bool,
        ambiguous: bool,
    ) -> EpisodeFileCoverage {
        EpisodeFileCoverage {
            has_present_file: present,
            has_missing_file: missing,
            has_unreadable_file: unreadable,
            ambiguous,
        }
    }

    #[test]
    fn ranks_unique_downloaded_episodes_and_seasons_without_special_or_ambiguous_coverage() {
        let episodes = vec![
            episode(
                "s1-versioned",
                "s1",
                "1",
                "2",
                Some("2024-01-02"),
                EpisodeType::Standard,
            ),
            episode(
                "s2-newest",
                "s2",
                "2",
                "1",
                Some("2024-01-02"),
                EpisodeType::Standard,
            ),
            episode(
                "special",
                "specials",
                "3",
                "1",
                Some("2024-01-03"),
                EpisodeType::Standard,
            ),
            episode(
                "season-zero",
                "s0",
                "0",
                "1",
                Some("2024-01-04"),
                EpisodeType::Standard,
            ),
            episode(
                "typed-special",
                "s1",
                "1",
                "3",
                Some("2024-01-04"),
                EpisodeType::Special,
            ),
            episode(
                "ambiguous",
                "s1",
                "1",
                "4",
                Some("2024-01-04"),
                EpisodeType::Standard,
            ),
            episode(
                "future",
                "s2",
                "2",
                "2",
                Some("2099-01-01"),
                EpisodeType::Standard,
            ),
            episode("unresolved", "s2", "2", "3", None, EpisodeType::Standard),
        ];
        let episode_coverage = HashMap::from([
            // A current version plus a stale missing version still means this
            // unique episode is downloaded; variants never consume a slot.
            (
                "s1-versioned".to_string(),
                coverage(true, true, false, false),
            ),
            ("s2-newest".to_string(), coverage(true, false, false, false)),
            ("special".to_string(), coverage(true, false, false, false)),
            (
                "season-zero".to_string(),
                coverage(true, false, false, false),
            ),
            (
                "typed-special".to_string(),
                coverage(true, false, false, false),
            ),
            ("ambiguous".to_string(), coverage(true, false, false, true)),
            ("future".to_string(), coverage(true, false, false, false)),
            (
                "unresolved".to_string(),
                coverage(true, false, false, false),
            ),
        ]);
        let inventory = LifecycleTitleEpisodeInventory {
            title_id: "title-1".to_string(),
            collections: vec![
                collection("s0", "0", CollectionType::Season),
                collection("s1", "1", CollectionType::Season),
                collection("s2", "2", CollectionType::Season),
                collection("specials", "3", CollectionType::Specials),
            ],
            episodes,
            scoped_files: vec![],
            file_availability: HashMap::new(),
            episode_coverage,
            retention_ranks: Mutex::default(),
        };

        let ranks = inventory.retention_ranks_at(
            DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")
                .expect("fixed timestamp parses")
                .with_timezone(&Utc),
        );

        // Equal air dates resolve by season then episode number; S2E1 precedes
        // S1E2. Stable ids make any remaining tie deterministic.
        assert_eq!(ranks.episode_positions.get("s2-newest"), Some(&1));
        assert_eq!(ranks.episode_positions.get("s1-versioned"), Some(&2));
        assert_eq!(ranks.season_positions.get("s2"), Some(&1));
        assert_eq!(ranks.season_positions.get("s1"), Some(&2));
        for ineligible in [
            "special",
            "season-zero",
            "typed-special",
            "ambiguous",
            "future",
            "unresolved",
        ] {
            assert!(
                !ranks.episode_positions.contains_key(ineligible),
                "{ineligible}"
            );
        }
        assert!(!ranks.season_positions.contains_key("s0"));
        assert!(!ranks.season_positions.contains_key("specials"));
        let cached = inventory.retention_ranks_at(
            DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")
                .expect("fixed timestamp parses")
                .with_timezone(&Utc),
        );
        assert!(Arc::ptr_eq(&ranks, &cached));
        let versioned = inventory
            .episodes
            .iter()
            .find(|episode| episode.id == "s1-versioned")
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(inventory.downloaded_episode_count(&versioned), (1, false));
    }

    #[test]
    fn completed_candidate_coverage_keeps_live_ranks_after_another_newer_deletion() {
        let collections = vec![
            collection("target-season", "1", CollectionType::Season),
            collection("other-season", "2", CollectionType::Season),
            collection("newest-season", "3", CollectionType::Season),
        ];
        let episodes = vec![
            episode(
                "completed-target",
                "target-season",
                "1",
                "1",
                Some("2024-01-01"),
                EpisodeType::Standard,
            ),
            episode(
                "other-newer-deleted",
                "other-season",
                "2",
                "1",
                Some("2024-01-02"),
                EpisodeType::Standard,
            ),
            episode(
                "live-newest",
                "newest-season",
                "3",
                "1",
                Some("2024-01-03"),
                EpisodeType::Standard,
            ),
        ];
        let at = DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")
            .expect("fixed timestamp parses")
            .with_timezone(&Utc);
        let before_other_candidate_finished = LifecycleTitleEpisodeInventory {
            title_id: "title-1".to_string(),
            collections: collections.clone(),
            episodes: episodes.clone(),
            scoped_files: vec![],
            file_availability: HashMap::new(),
            episode_coverage: HashMap::from([
                (
                    "completed-target".to_string(),
                    coverage(true, false, false, false),
                ),
                (
                    "other-newer-deleted".to_string(),
                    coverage(true, false, false, false),
                ),
                (
                    "live-newest".to_string(),
                    coverage(true, false, false, false),
                ),
            ]),
            retention_ranks: Mutex::default(),
        };
        assert_eq!(
            before_other_candidate_finished
                .retention_ranks_at(at)
                .season_positions
                .get("target-season"),
            Some(&3),
            "the candidate originally matched delete-after-the-newest-two"
        );

        let inventory = LifecycleTitleEpisodeInventory {
            title_id: "title-1".to_string(),
            collections,
            episodes,
            scoped_files: vec![],
            file_availability: HashMap::new(),
            episode_coverage: HashMap::from([
                // This candidate's file and a different, newer candidate's
                // file have both gone. Only the former is in this journal.
                (
                    "completed-target".to_string(),
                    coverage(false, true, false, false),
                ),
                (
                    "other-newer-deleted".to_string(),
                    coverage(false, true, false, false),
                ),
                (
                    "live-newest".to_string(),
                    coverage(true, false, false, false),
                ),
            ]),
            retention_ranks: Mutex::default(),
        };
        let live = inventory.retention_ranks_at(at);
        assert_eq!(live.episode_positions.get("live-newest"), Some(&1));
        assert!(!live.episode_positions.contains_key("completed-target"));
        assert!(!live.episode_positions.contains_key("other-newer-deleted"));

        let restored = inventory.retention_ranks_with_completed_episode_coverage(
            at,
            &BTreeSet::from(["completed-target".to_string()]),
        );
        assert_eq!(restored.episode_positions.get("live-newest"), Some(&1));
        assert_eq!(
            restored.episode_positions.get("completed-target"),
            Some(&2),
            "the target is now within the newest two and must not use its frozen rank three"
        );
        assert_eq!(restored.season_positions.get("newest-season"), Some(&1));
        assert_eq!(restored.season_positions.get("target-season"), Some(&2));
        assert!(
            restored
                .season_positions
                .get("target-season")
                .is_some_and(|rank| *rank <= 2),
            "the live removal by another candidate protects this target from a > 2 policy"
        );
    }
}
