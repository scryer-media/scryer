//! Scoped maintenance identity and current catalog membership. Never infer a
//! missing child identity from its parent title.
use crate::types::TitleMediaFile;
use crate::{AppError, AppResult, AppUseCase};
use scryer_domain::{
    Collection, CollectionType, Episode, MaintenanceRuleExclusion,
    MaintenanceRuleSubjectKind as Scope, Title,
};
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub(crate) struct MaintenanceSubject {
    pub kind: Scope,
    pub id: String,
    pub collection: Option<Collection>,
    pub episodes: Vec<Episode>,
    pub files: Vec<TitleMediaFile>,
    pub episode_file_count: i64,
    pub ambiguous_files: bool,
    pub monitored: bool,
    pub label: String,
}

impl MaintenanceSubject {
    pub fn excluded_by(&self, title_id: &str, exclusions: &[MaintenanceRuleExclusion]) -> bool {
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
    pub(crate) async fn maintenance_subjects_for_title(
        &self,
        kind: Scope,
        title: &Title,
        title_files: &[TitleMediaFile],
    ) -> AppResult<Vec<MaintenanceSubject>> {
        if kind == Scope::Title {
            return Ok(vec![MaintenanceSubject {
                kind,
                id: title.id.clone(),
                collection: None,
                episodes: vec![],
                files: title_files.to_vec(),
                episode_file_count: 0,
                ambiguous_files: false,
                monitored: title.monitored,
                label: title.name.clone(),
            }]);
        }
        if title.facet == scryer_domain::MediaFacet::Movie {
            return Ok(vec![]);
        }
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
        if episodes.iter().any(|episode| episode.title_id != title.id)
            || collections
                .iter()
                .any(|collection| collection.title_id != title.id)
        {
            return Err(AppError::Repository(
                "maintenance subject ownership mismatch".into(),
            ));
        }
        if episodes.iter().any(|episode| {
            episode
                .collection_id
                .as_ref()
                .is_some_and(|id| !collections.iter().any(|collection| &collection.id == id))
        }) {
            return Err(AppError::Repository(
                "maintenance episode collection ownership mismatch".into(),
            ));
        }
        let mut subjects = Vec::new();
        if kind == Scope::Season {
            for collection in collections.iter().filter(|collection| {
                matches!(
                    collection.collection_type,
                    CollectionType::Season | CollectionType::Specials
                )
            }) {
                subjects.push(MaintenanceSubject {
                    kind,
                    id: collection.id.clone(),
                    collection: Some(collection.clone()),
                    episodes: episodes
                        .iter()
                        .filter(|episode| episode.collection_id.as_deref() == Some(&collection.id))
                        .cloned()
                        .collect(),
                    files: vec![],
                    episode_file_count: 0,
                    ambiguous_files: false,
                    monitored: collection.monitored,
                    label: format!("Season {}", collection.collection_index),
                });
            }
        } else {
            for episode in &episodes {
                let collection = collections
                    .iter()
                    .find(|collection| Some(&collection.id) == episode.collection_id.as_ref())
                    .cloned();
                subjects.push(MaintenanceSubject {
                    kind,
                    id: episode.id.clone(),
                    collection,
                    episodes: vec![episode.clone()],
                    files: vec![],
                    episode_file_count: 0,
                    ambiguous_files: false,
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
        // Bound the relation query. Keep complete coverage returned for each file,
        // including associations outside the selected episode batch.
        let mut scoped_files = std::collections::HashMap::new();
        for batch in episodes.chunks(100) {
            let ids: Vec<String> = batch.iter().map(|episode| episode.id.clone()).collect();
            for file in self
                .services
                .library
                .media_files
                .list_live_media_files_for_episode_ids(&title.id, &ids)
                .await?
            {
                scoped_files.insert(file.media_file.id.clone(), file);
            }
        }
        for subject in &mut subjects {
            let ids: HashSet<&str> = subject
                .episodes
                .iter()
                .map(|episode| episode.id.as_str())
                .collect();
            let mut covered = HashSet::new();
            for scoped in scoped_files.values() {
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
                covered.extend(
                    scoped
                        .episode_ids
                        .iter()
                        .filter(|id| ids.contains(id.as_str())),
                );
                subject.files.push(scoped.media_file.clone());
            }
            subject.episode_file_count = covered.len() as i64;
            subject.files.sort_by(|a, b| a.id.cmp(&b.id));
        }
        Ok(subjects)
    }

    pub(crate) async fn maintenance_resolve_subject(
        &self,
        kind: &str,
        subject_id: &str,
        title: &Title,
        files: &[TitleMediaFile],
    ) -> AppResult<MaintenanceSubject> {
        let kind = Scope::parse_storage(kind)
            .ok_or_else(|| AppError::Validation("invalid maintenance subject kind".into()))?;
        if subject_id.is_empty() {
            return Err(AppError::Validation(
                "maintenance subject ID is required".into(),
            ));
        }
        self.maintenance_subjects_for_title(kind, title, files)
            .await?
            .into_iter()
            .find(|subject| subject.id == subject_id)
            .ok_or_else(|| {
                AppError::NotFound("maintenance subject no longer belongs to this title".into())
            })
    }
}

impl AppUseCase {
    /// At most one subject load per (scope, title) for a bounded results page.
    pub(crate) async fn maintenance_subject_summaries(
        &self,
        keys: impl Iterator<Item = (String, String)>,
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
            for subject in self
                .maintenance_subjects_for_title(kind, &title, &files)
                .await?
            {
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
