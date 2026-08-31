use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use scryer_domain::{
    Collection, DomainEventPayload, Episode, ImportType, MediaFacet, MediaFileRenamedEventData,
    Title, User,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use futures_util::stream::{StreamExt, TryStreamExt};

use crate::activity::NotificationMediaUpdate;
use crate::catalog_workflow::{HydrationSource, HydrationTarget};
use crate::domain_events::{
    created_media_update, deleted_media_update, new_title_domain_event, title_context_snapshot,
};
use crate::facet_handler::{RenameFacetSettings, rename_facet_settings};
use crate::media::release_labels::resolve_release_labels_from_analysis;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{
    AppError, AppResult, AppUseCase, ClientJobLocator, CollectionUpdate,
    DEFAULT_FOLDER_TEMPLATE_ANIME, DEFAULT_FOLDER_TEMPLATE_MOVIE, DEFAULT_FOLDER_TEMPLATE_SERIES,
    DEFAULT_SEASON_FOLDER_TEMPLATE, DEFAULT_SPECIALS_FOLDER_TEMPLATE, FOLDER_TEMPLATE_KEY,
    ParsedEpisodeMetadata, ParsedReleaseMetadata, SEASON_FOLDER_TEMPLATE_KEY,
    SPECIALS_FOLDER_TEMPLATE_KEY, TitleMediaFile, parse_release_metadata,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenameWriteAction {
    Noop,
    Move,
    Replace,
    Skip,
    Error,
}

impl RenameWriteAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Move => "move",
            Self::Replace => "replace",
            Self::Skip => "skip",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenameApplyStatus {
    Applied,
    Skipped,
    Failed,
}

impl RenameApplyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RenameCollisionPolicy {
    #[default]
    Skip,
    Error,
    ReplaceIfBetter,
}

impl RenameCollisionPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Error => "error",
            Self::ReplaceIfBetter => "replace_if_better",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RenameMissingMetadataPolicy {
    Skip,
    #[default]
    FallbackTitle,
}

impl RenameMissingMetadataPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::FallbackTitle => "fallback_title",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlanItem {
    pub collection_id: Option<String>,
    pub media_file_id: Option<String>,
    pub series_movie_link_ids: Vec<String>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub normalized_filename: Option<String>,
    pub collision: bool,
    pub reason_code: String,
    pub write_action: RenameWriteAction,
    pub source_size_bytes: Option<u64>,
    pub source_mtime_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlan {
    pub facet: MediaFacet,
    pub title_id: Option<String>,
    pub template: String,
    pub collision_policy: RenameCollisionPolicy,
    pub missing_metadata_policy: RenameMissingMetadataPolicy,
    pub fingerprint: String,
    pub total: usize,
    pub renamable: usize,
    pub noop: usize,
    pub conflicts: usize,
    pub errors: usize,
    pub items: Vec<RenamePlanItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameApplyItemResult {
    pub collection_id: Option<String>,
    pub media_file_id: Option<String>,
    pub series_movie_link_ids: Vec<String>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub final_path: Option<String>,
    pub write_action: RenameWriteAction,
    pub status: RenameApplyStatus,
    pub reason_code: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameApplyResult {
    pub plan_fingerprint: String,
    pub total: usize,
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub items: Vec<RenameApplyItemResult>,
}

#[async_trait]
pub trait LibraryRenamer: Send + Sync {
    async fn validate_targets(&self, plan: &RenamePlan) -> AppResult<()>;
    /// Applies the plan, then puts the configured permissions on each moved
    /// file. Permissions are resolved by the caller because only it can read
    /// settings; a mover that guessed from the source would let a move change
    /// access that an operator deliberately configured.
    async fn apply_plan(
        &self,
        plan: &RenamePlan,
        permissions: &crate::ImportFilePermissions,
    ) -> AppResult<Vec<RenameApplyItemResult>>;
    async fn rollback(
        &self,
        applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>>;
}

#[derive(Default)]
pub struct NullLibraryRenamer;

#[async_trait]
impl LibraryRenamer for NullLibraryRenamer {
    async fn validate_targets(&self, _plan: &RenamePlan) -> AppResult<()> {
        Err(AppError::Repository(
            "library renamer is not configured".into(),
        ))
    }

    async fn apply_plan(
        &self,
        _plan: &RenamePlan,
        _permissions: &crate::ImportFilePermissions,
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        Err(AppError::Repository(
            "library renamer is not configured".into(),
        ))
    }

    async fn rollback(
        &self,
        _applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        Ok(Vec::new())
    }
}

const RENAME_COLLISION_POLICY_KEY: &str = "rename.collision_policy";
const RENAME_COLLISION_POLICY_GLOBAL_KEY: &str = "rename.collision_policy.global";
const RENAME_MISSING_METADATA_POLICY_KEY: &str = "rename.missing_metadata_policy";
const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY: &str = "rename.missing_metadata_policy.global";
const DEFAULT_COLLISION_POLICY: RenameCollisionPolicy = RenameCollisionPolicy::Skip;
const DEFAULT_MISSING_METADATA_POLICY: RenameMissingMetadataPolicy =
    RenameMissingMetadataPolicy::FallbackTitle;
const GENERATED_COMPONENT_MAX_BYTES: usize = 240;
const GENERATED_COMPONENT_SUFFIX_RESERVE_BYTES: usize = 24;
const MAX_RENAME_TEMPLATE_PADDING_WIDTH: usize = 240;

#[derive(Default)]
struct RenamePersistenceState {
    media_file_updated: bool,
}

struct RenamePersistenceFailure {
    error: AppError,
    state: RenamePersistenceState,
}

struct RenameRollbackOutcome {
    fully_restored: bool,
    detail: String,
}

/// Facet-level rename settings, resolved once per plan.
///
/// Every field here is constant across the titles in one plan, so reading them
/// per title only repeats the same settings queries. The media root is the one
/// path input that genuinely varies, and it stays a per-title lookup.
/// Titles planned at once inside one batched preview.
///
/// The per-title fan-out this batch replaced ran four requests at a time, so
/// the batch has to overlap at least as much to stay faster than it. The cap
/// keeps a large selection from monopolizing the database pool.
const RENAME_PREVIEW_TITLE_CONCURRENCY: usize = 8;

#[derive(Clone)]
struct RenamePlanSettings {
    template: String,
    folder_template: String,
    season_folder_template: String,
    specials_folder_template: String,
    collision_policy: RenameCollisionPolicy,
    missing_metadata_policy: RenameMissingMetadataPolicy,
}

impl AppUseCase {
    pub async fn preview_rename_for_title(
        &self,
        actor: &User,
        title_id: &str,
        facet: MediaFacet,
    ) -> AppResult<RenamePlan> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if title.facet != facet {
            return Err(AppError::Validation(
                "requested facet does not match title facet".into(),
            ));
        }
        if !self.resolve_rename_enabled(&facet).await? {
            return Err(AppError::Validation("renamer_disabled".into()));
        }

        let settings = self.read_rename_plan_settings(&facet).await?;
        let effective_languages = self
            .resolve_metadata_languages_for_titles(std::slice::from_ref(&title))
            .await;
        self.build_rename_plan_for_titles(
            title.facet.clone(),
            std::slice::from_ref(&title),
            Some(title.id.clone()),
            settings,
            &effective_languages,
        )
        .await
    }

    /// Previews each title's rename plan, resolving shared settings once.
    ///
    /// Titles keep their own plan (and their own fingerprint, which apply still
    /// validates per title); batching only removes the per-request title load,
    /// permission check, and settings reads that a preview-per-title fan-out
    /// repeats for every title.
    pub async fn preview_rename_for_titles(
        &self,
        actor: &User,
        title_ids: &[String],
        facet: MediaFacet,
    ) -> AppResult<Vec<RenamePlan>> {
        // Authorize every title before reading any settings, so an actor who
        // cannot see these titles learns nothing about the facet's renamer.
        // This keeps the single-title ordering: load, authorize, then check.
        let mut titles = Vec::with_capacity(title_ids.len());
        for title_id in title_ids {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
            if title.facet != facet {
                return Err(AppError::Validation(
                    "requested facet does not match title facet".into(),
                ));
            }
            titles.push(title);
        }

        if !self.resolve_rename_enabled(&facet).await? {
            return Err(AppError::Validation("renamer_disabled".into()));
        }
        let settings = self.read_rename_plan_settings(&facet).await?;
        let effective_languages = self.resolve_metadata_languages_for_titles(&titles).await;

        // Each title plans against its own state, so they are independent and
        // run concurrently. Planning one title at a time would serialize work
        // the per-title callers used to overlap, making the batch slower than
        // the fan-out it replaced however little per-title work it saves.
        futures_util::stream::iter(titles.into_iter().map(|title| {
            let settings = settings.clone();
            let effective_languages = effective_languages.clone();
            async move {
                self.build_rename_plan_for_titles(
                    title.facet.clone(),
                    std::slice::from_ref(&title),
                    Some(title.id.clone()),
                    settings,
                    &effective_languages,
                )
                .await
            }
        }))
        .buffered(RENAME_PREVIEW_TITLE_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await
    }

    pub async fn preview_rename_for_facet(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<RenamePlan> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        if !self.resolve_rename_enabled(&facet).await? {
            return Err(AppError::Validation("renamer_disabled".into()));
        }

        let settings = self.read_rename_plan_settings(&facet).await?;
        let titles = self
            .services
            .catalog
            .titles
            .list(Some(facet.clone()), None)
            .await?;
        // A facet plan must never reach into a library the actor cannot manage:
        // holding a catalog-settings permission is not the same as being allowed
        // to rewrite someone else's library.
        let mut manageable = Vec::with_capacity(titles.len());
        for title in titles {
            if self
                .require_library_permission(
                    actor,
                    &title.library_id,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await
                .is_ok()
            {
                manageable.push(title);
            }
        }
        let mut titles = manageable;
        titles.sort_by(|left, right| left.id.cmp(&right.id));
        let effective_languages = self.resolve_metadata_languages_for_titles(&titles).await;
        self.build_rename_plan_for_titles(facet, &titles, None, settings, &effective_languages)
            .await
    }

    pub async fn apply_rename_for_title(
        &self,
        actor: &User,
        title_id: &str,
        facet: MediaFacet,
        plan_fingerprint: &str,
    ) -> AppResult<RenameApplyResult> {
        // Keep preview read-only. The execution endpoint is the only place a
        // stale language selection may refresh and persist catalog metadata.
        self.preview_rename_for_title(actor, title_id, facet.clone())
            .await?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        let effective_languages = self
            .resolve_metadata_languages_for_titles(std::slice::from_ref(&title))
            .await;
        let effective_language = effective_languages
            .get(&title.id)
            .map(String::as_str)
            .unwrap_or("eng");
        self.refresh_rename_title_metadata_if_stale(&title, effective_language)
            .await;
        let preview = self
            .preview_rename_for_title(actor, title_id, facet)
            .await?;
        self.apply_previewed_rename_plan(actor, preview, plan_fingerprint)
            .await
    }

    pub async fn apply_rename_for_facet(
        &self,
        actor: &User,
        facet: MediaFacet,
        plan_fingerprint: &str,
    ) -> AppResult<RenameApplyResult> {
        // Authorize and build the read-only plan before issuing any execution
        // refreshes, then only refresh titles the actor may actually rename.
        self.preview_rename_for_facet(actor, facet.clone()).await?;
        let titles = self
            .services
            .catalog
            .titles
            .list(Some(facet.clone()), None)
            .await?;
        let manageable = futures_util::future::join_all(titles.into_iter().map(|title| async {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await
            .is_ok()
            .then_some(title)
        }))
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let effective_languages = self
            .resolve_metadata_languages_for_titles(&manageable)
            .await;
        for title in &manageable {
            let effective_language = effective_languages
                .get(&title.id)
                .map(String::as_str)
                .unwrap_or("eng");
            self.refresh_rename_title_metadata_if_stale(title, effective_language)
                .await;
        }
        let preview = self.preview_rename_for_facet(actor, facet).await?;
        self.apply_previewed_rename_plan(actor, preview, plan_fingerprint)
            .await
    }

    pub async fn record_rename_apply_audit(
        &self,
        actor: &User,
        operation: &str,
        facet: &str,
        title_id: Option<&str>,
        idempotency_key: Option<&str>,
        result: &RenameApplyResult,
    ) -> AppResult<()> {
        if let Some(title_id) = title_id {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        } else {
            self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let plan_fingerprint = result.plan_fingerprint.clone();
        let progress_json = serde_json::json!({
            "operation": operation,
            "facet": facet,
            "title_id": title_id,
            "idempotency_key": idempotency_key,
            "plan_fingerprint": plan_fingerprint.clone(),
            "total": result.total,
            "applied": result.applied,
            "skipped": result.skipped,
            "failed": result.failed,
        })
        .to_string();

        let _ = self
            .services
            .workflow
            .workflow_operations
            .create_workflow_operation(
                operation.to_string(),
                "completed".to_string(),
                Some(actor.id.clone()),
                Some(progress_json),
                Some(now.clone()),
                Some(now),
            )
            .await?;

        let source_ref = if let Some(key) = idempotency_key {
            format!("{operation}:{key}")
        } else if let Some(title_id) = title_id {
            format!("{operation}:title:{title_id}:{plan_fingerprint}")
        } else {
            format!("{operation}:facet:{facet}:{plan_fingerprint}")
        };
        let payload_json = serde_json::to_string(result).unwrap_or_else(|_| {
            "{\"error\":\"failed_to_serialize_rename_apply_result\"}".to_string()
        });

        let _ = self
            .services
            .workflow
            .imports
            .queue_import_request(
                ClientJobLocator::new(None, "scryer_rename", source_ref),
                ImportType::RenameApplyResult.as_str().to_string(),
                payload_json,
            )
            .await?;

        Ok(())
    }

    async fn read_rename_plan_settings(&self, facet: &MediaFacet) -> AppResult<RenamePlanSettings> {
        let facet_settings = rename_facet_settings(facet);
        Ok(RenamePlanSettings {
            template: self.resolve_rename_template(facet).await?,
            folder_template: self.read_folder_template(facet_settings).await?,
            season_folder_template: normalize_season_folder_template_or_default(
                self.read_setting_string_value(
                    SEASON_FOLDER_TEMPLATE_KEY,
                    Some(facet_settings.scope_id),
                )
                .await?,
            ),
            specials_folder_template: normalize_specials_folder_template_or_default(
                self.read_setting_string_value(
                    SPECIALS_FOLDER_TEMPLATE_KEY,
                    Some(facet_settings.scope_id),
                )
                .await?,
            ),
            collision_policy: self.read_collision_policy(facet_settings).await?,
            missing_metadata_policy: self.read_missing_metadata_policy(facet_settings).await?,
        })
    }

    async fn apply_previewed_rename_plan(
        &self,
        actor: &User,
        preview: RenamePlan,
        plan_fingerprint: &str,
    ) -> AppResult<RenameApplyResult> {
        if preview.fingerprint != plan_fingerprint {
            return Err(AppError::Validation("rename_stale_plan".into()));
        }

        self.apply_rename_plan(actor, preview).await
    }

    async fn apply_rename_plan(
        &self,
        actor: &User,
        preview: RenamePlan,
    ) -> AppResult<RenameApplyResult> {
        self.preflight_rename_folder_ownership(&preview).await?;
        self.services
            .library
            .library_renamer
            .validate_targets(&preview)
            .await?;

        // Configured permissions win over whatever the files carried before, the
        // way Sonarr applies ChmodFolder/ChownGroup after every transfer rather
        // than preserving the source's mode.
        // Plans are per title, so the library override that applies is the one
        // on the title being renamed rather than the facet default.
        let library_id = match preview.title_id.as_deref() {
            Some(title_id) => self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .map(|title| title.library_id),
            None => first_library_id_in_plan(self, &preview).await,
        };
        let permissions = self
            .resolve_import_file_permissions(library_id.as_deref(), &preview.facet)
            .await?;

        let mut item_results = self
            .services
            .library
            .library_renamer
            .apply_plan(&preview, &permissions)
            .await?;
        let mut applied = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;

        for item in &mut item_results {
            match item.status {
                RenameApplyStatus::Applied => {
                    if let Some(final_path) = item.final_path.clone()
                        && let Err(failure) =
                            self.persist_rename_item_paths(item, &final_path).await
                    {
                        let rollback = self
                            .rollback_rename_item_after_db_failure(item, &failure.state)
                            .await;

                        item.status = RenameApplyStatus::Failed;
                        item.reason_code = "db_update_failed".into();
                        item.error_message =
                            Some(format!("{}; {}", failure.error, rollback.detail));
                        if rollback.fully_restored {
                            item.final_path = Some(item.current_path.clone());
                        }
                        failed += 1;
                        continue;
                    }
                    applied += 1;
                }
                RenameApplyStatus::Skipped => {
                    skipped += 1;
                }
                RenameApplyStatus::Failed => {
                    failed += 1;
                }
            }
        }

        let result = RenameApplyResult {
            plan_fingerprint: preview.fingerprint.clone(),
            total: item_results.len(),
            applied,
            skipped,
            failed,
            items: item_results,
        };

        self.emit_rename_notifications(actor, &result.items).await;

        Ok(result)
    }

    async fn preflight_rename_folder_ownership(&self, preview: &RenamePlan) -> AppResult<()> {
        for item in &preview.items {
            if !matches!(
                item.write_action,
                RenameWriteAction::Move | RenameWriteAction::Replace
            ) {
                continue;
            }
            let Some(proposed_path) = item.proposed_path.as_deref() else {
                continue;
            };
            let probe = RenameApplyItemResult {
                collection_id: item.collection_id.clone(),
                media_file_id: item.media_file_id.clone(),
                series_movie_link_ids: item.series_movie_link_ids.clone(),
                current_path: item.current_path.clone(),
                proposed_path: item.proposed_path.clone(),
                final_path: None,
                write_action: item.write_action.clone(),
                status: RenameApplyStatus::Skipped,
                reason_code: String::new(),
                error_message: None,
            };
            let Some(title) = self.resolve_title_for_rename_item(&probe).await? else {
                continue;
            };
            let use_season_folders = self.resolve_use_season_folders(&title).await?;
            let Some(folder_path) = infer_title_folder_path_after_rename(
                &title,
                use_season_folders,
                &item.current_path,
                proposed_path,
            ) else {
                continue;
            };
            crate::folder_ownership::ensure_folder_move_available_to_title(
                self,
                &title,
                &stored_path_to_path_buf(&folder_path),
            )
            .await?;
        }
        Ok(())
    }

    async fn library_id_for_rename_item(&self, item: &RenameApplyItemResult) -> Option<String> {
        self.resolve_title_for_rename_item(item)
            .await
            .ok()
            .flatten()
            .map(|title| title.library_id)
    }

    async fn emit_rename_notifications(&self, actor: &User, items: &[RenameApplyItemResult]) {
        let mut grouped: HashMap<String, (Title, Vec<NotificationMediaUpdate>, Vec<String>)> =
            HashMap::new();
        let mut cached_episode_ids_by_file: HashMap<String, Vec<String>> = HashMap::new();

        for item in items {
            if !matches!(item.status, RenameApplyStatus::Applied) {
                continue;
            }

            let Some(final_path) = item.final_path.clone() else {
                continue;
            };

            let title = match self.resolve_title_for_rename_item(item).await {
                Ok(Some(title)) => title,
                Ok(None) => continue,
                Err(error) => {
                    warn!(
                        error = %error,
                        current_path = item.current_path.as_str(),
                        "failed to resolve title for rename notification"
                    );
                    continue;
                }
            };

            let episode_ids = if let Some(media_file_id) = item.media_file_id.as_deref() {
                if let Some(cached) = cached_episode_ids_by_file.get(media_file_id) {
                    cached.clone()
                } else {
                    let ids = self
                        .services
                        .library
                        .media_files
                        .list_media_files_for_title(&title.id)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|media_file| media_file.id == media_file_id)
                        .filter_map(|media_file| media_file.episode_id)
                        .collect::<Vec<_>>();
                    cached_episode_ids_by_file.insert(media_file_id.to_string(), ids.clone());
                    ids
                }
            } else {
                Vec::new()
            };

            let entry = grouped
                .entry(title.id.clone())
                .or_insert_with(|| (title.clone(), Vec::new(), Vec::new()));
            entry
                .1
                .push(NotificationMediaUpdate::deleted(item.current_path.clone()));
            entry.1.push(NotificationMediaUpdate::created(final_path));
            for episode_id in episode_ids {
                if !entry.2.contains(&episode_id) {
                    entry.2.push(episode_id);
                }
            }
        }

        for (_title_id, (title, updates, episode_ids)) in grouped {
            if updates.is_empty() {
                continue;
            }

            let renamed_files = updates
                .iter()
                .filter(|u| u.update_type == "created")
                .count();
            let domain_updates = updates
                .iter()
                .map(|update| match update.update_type {
                    "deleted" => deleted_media_update(update.path.clone()),
                    _ => created_media_update(update.path.clone()),
                })
                .collect();
            if let Err(error) = self
                .append_domain_event(new_title_domain_event(
                    actor,
                    &title,
                    DomainEventPayload::MediaFileRenamed(MediaFileRenamedEventData {
                        title: title_context_snapshot(&title),
                        media_updates: domain_updates,
                        renamed_count: renamed_files as i32,
                        episode_ids,
                    }),
                ))
                .await
            {
                warn!(
                    error = %error,
                    title = title.name.as_str(),
                    "failed to append media file renamed domain event"
                );
            }
        }
    }

    async fn resolve_title_for_rename_item(
        &self,
        item: &RenameApplyItemResult,
    ) -> AppResult<Option<Title>> {
        let title_id = if let Some(media_file_id) = item.media_file_id.as_deref() {
            self.services
                .library
                .media_files
                .get_media_file_by_id(media_file_id)
                .await?
                .map(|file| file.title_id)
        } else if let Some(collection_id) = item.collection_id.as_deref() {
            self.services
                .catalog
                .shows
                .get_collection_by_id(collection_id)
                .await?
                .map(|collection| collection.title_id)
        } else {
            None
        };

        match title_id {
            Some(title_id) => self.services.catalog.titles.get_by_id(&title_id).await,
            None => Ok(None),
        }
    }

    async fn persist_rename_item_paths(
        &self,
        item: &RenameApplyItemResult,
        final_path: &str,
    ) -> Result<(), RenamePersistenceFailure> {
        let mut state = RenamePersistenceState::default();

        if let Some(media_file_id) = item.media_file_id.as_deref()
            && let Err(error) = self
                .services
                .library
                .media_files
                .update_media_file_path(media_file_id, final_path)
                .await
        {
            return Err(RenamePersistenceFailure { error, state });
        } else if item.media_file_id.is_some() {
            state.media_file_updated = true;
        }

        if let Some(collection_id) = item.collection_id.as_deref()
            && let Err(error) = self
                .services
                .catalog
                .shows
                .update_collection(
                    collection_id,
                    CollectionUpdate {
                        ordered_path: Some(final_path.to_string()),
                        ..Default::default()
                    },
                )
                .await
        {
            return Err(RenamePersistenceFailure { error, state });
        }

        let title = match self.resolve_title_for_rename_item(item).await {
            Ok(title) => title,
            Err(error) => return Err(RenamePersistenceFailure { error, state }),
        };
        if let Some(title) = title {
            let use_season_folders = match self.resolve_use_season_folders(&title).await {
                Ok(value) => value,
                Err(error) => return Err(RenamePersistenceFailure { error, state }),
            };
            if let Some(folder_path) = infer_title_folder_path_after_rename(
                &title,
                use_season_folders,
                &item.current_path,
                final_path,
            ) && let Err(error) = self
                .services
                .catalog
                .titles
                .set_folder_path(&title.id, &folder_path)
                .await
            {
                return Err(RenamePersistenceFailure { error, state });
            }
        }

        Ok(())
    }

    async fn rollback_rename_item_after_db_failure(
        &self,
        item: &RenameApplyItemResult,
        state: &RenamePersistenceState,
    ) -> RenameRollbackOutcome {
        let mut details = Vec::new();
        let mut fully_restored = true;
        let mut filesystem_restored = false;

        match item.write_action {
            RenameWriteAction::Move => match self
                .services
                .library
                .library_renamer
                .rollback(std::slice::from_ref(item))
                .await
            {
                Ok(_) => {
                    filesystem_restored = true;
                }
                Err(error) => {
                    fully_restored = false;
                    details.push(format!("filesystem rollback failed: {error}"));
                }
            },
            _ => {
                fully_restored = false;
                details.push("filesystem rollback unavailable for this write action".to_string());
            }
        }

        if filesystem_restored
            && state.media_file_updated
            && let Some(media_file_id) = item.media_file_id.as_deref()
            && let Err(error) = self
                .services
                .library
                .media_files
                .update_media_file_path(media_file_id, &item.current_path)
                .await
        {
            fully_restored = false;
            details.push(format!("media file rollback failed: {error}"));
        }

        if details.is_empty() {
            RenameRollbackOutcome {
                fully_restored,
                detail: "rollback succeeded".to_string(),
            }
        } else {
            RenameRollbackOutcome {
                fully_restored,
                detail: format!("rollback failed: {}", details.join("; ")),
            }
        }
    }

    async fn build_rename_plan_for_titles(
        &self,
        facet: MediaFacet,
        titles: &[Title],
        title_id: Option<String>,
        settings: RenamePlanSettings,
        effective_languages: &HashMap<String, String>,
    ) -> AppResult<RenamePlan> {
        let mut planning = RenamePlanningState::default();
        let mut items = Vec::new();
        for title in titles {
            let effective_language = effective_languages
                .get(&title.id)
                .map(String::as_str)
                .unwrap_or("eng");
            let mut title_items = self
                .build_rename_plan_items_for_title(
                    title,
                    effective_language,
                    &settings,
                    &mut planning,
                )
                .await?;
            items.append(&mut title_items);
        }

        Ok(build_rename_plan_from_items(
            facet,
            title_id,
            settings.template,
            settings.collision_policy,
            settings.missing_metadata_policy,
            items,
        ))
    }

    async fn build_rename_plan_items_for_title(
        &self,
        title: &Title,
        _effective_language: &str,
        settings: &RenamePlanSettings,
        planning: &mut RenamePlanningState,
    ) -> AppResult<Vec<RenamePlanItem>> {
        let title = title.clone();
        // Only the media root varies per title; every template and policy came
        // from `settings`, which was resolved once for the whole plan.
        let media_root = self.title_root_folder_path_override(&title).await?;
        let use_season_folders = self.resolve_use_season_folders(&title).await?;
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;
        let media_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await?
            .into_iter()
            .filter(|file| file.role.is_primary())
            .collect::<Vec<_>>();
        let episodes = match title.facet {
            MediaFacet::Movie => Vec::new(),
            MediaFacet::Series | MediaFacet::Anime => {
                self.services
                    .catalog
                    .shows
                    .list_episodes_for_title(&title.id)
                    .await?
            }
        };

        // Item building stats every source file and probes every destination,
        // so it runs on the blocking pool instead of stalling a runtime worker
        // for the whole title.
        let items = {
            let title = title.clone();
            let settings = settings.clone();
            let mut owned_planning = std::mem::take(planning);
            let (built, returned_planning) = tokio::task::spawn_blocking(move || {
                let items = match title.facet.clone() {
                    MediaFacet::Movie => {
                        let mut options = MovieRenamePlanOptions {
                            media_root: &media_root,
                            folder_template: &settings.folder_template,
                            template: &settings.template,
                            missing_metadata_policy: &settings.missing_metadata_policy,
                            planning: &mut owned_planning,
                        };
                        build_movie_rename_plan_items(
                            &title,
                            collections,
                            media_files,
                            &mut options,
                        )
                    }
                    MediaFacet::Series | MediaFacet::Anime => {
                        build_series_rename_plan_items_from_media_files(
                            &title,
                            use_season_folders,
                            collections,
                            episodes,
                            media_files,
                            &media_root,
                            &settings.folder_template,
                            &settings.season_folder_template,
                            &settings.specials_folder_template,
                            &settings.template,
                            &settings.missing_metadata_policy,
                            &mut owned_planning,
                        )
                    }
                };
                (items, owned_planning)
            })
            .await
            .map_err(|error| {
                AppError::Repository(format!("rename plan build task failed to join: {error}"))
            })?;
            *planning = returned_planning;
            built
        };

        self.normalize_existing_rename_collisions(items).await
    }

    async fn refresh_rename_title_metadata_if_stale(
        &self,
        title: &Title,
        effective_language: &str,
    ) -> Title {
        let metadata_language = title
            .metadata_language
            .as_deref()
            .and_then(crate::normalize_metadata_language_code);
        if metadata_language.as_deref() == Some(effective_language) {
            return title.clone();
        }

        match self
            .hydrate_title_single_apq_with_language(
                HydrationTarget {
                    title: title.clone(),
                    requested_tvdb_id: None,
                    requested_movie_ref: None,
                    sync_wanted_after_completion: false,
                    source: HydrationSource::Interactive,
                },
                effective_language,
            )
            .await
        {
            Ok(refreshed) => refreshed,
            Err(error) => {
                warn!(
                    title_id = %title.id,
                    title_name = %title.name,
                    effective_language,
                    persisted_metadata_language = ?metadata_language,
                    error = %error,
                    "rename metadata refresh failed; using persisted title metadata"
                );
                title.clone()
            }
        }
    }

    async fn normalize_existing_rename_collisions(
        &self,
        items: Vec<RenamePlanItem>,
    ) -> AppResult<Vec<RenamePlanItem>> {
        // One lookup per store for the whole title instead of two point queries
        // per item; the destination probes were already memoized while building.
        let lookup_paths = items
            .iter()
            .filter_map(|item| {
                let proposed_path = item.proposed_path.as_ref()?;
                (!crate::stored_paths::paths_match(proposed_path, &item.current_path))
                    .then(|| proposed_path.clone())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let media_file_cache = self
            .services
            .library
            .media_files
            .list_media_files_by_paths(&lookup_paths)
            .await?
            .into_iter()
            .collect::<HashMap<String, TitleMediaFile>>();
        let mut collection_cache = HashMap::<String, Collection>::new();
        for collection in self
            .services
            .catalog
            .shows
            .list_collections_by_ordered_paths(&lookup_paths)
            .await?
        {
            // `ordered_path` has no unique constraint, and the per-path query
            // this replaced resolved duplicates with `ORDER BY id ASC LIMIT 1`.
            // Rows arrive id-ascending, so keep the first and ignore the rest.
            if let Some(ordered_path) = collection.ordered_path.clone() {
                collection_cache.entry(ordered_path).or_insert(collection);
            }
        }

        let mut out = Vec::with_capacity(items.len());

        for mut item in items {
            let Some(proposed_path) = item.proposed_path.clone() else {
                out.push(item);
                continue;
            };

            if crate::stored_paths::paths_match(&proposed_path, &item.current_path) {
                out.push(item);
                continue;
            }

            let tracked_media_file = media_file_cache.get(&proposed_path);
            let tracked_collection = collection_cache.get(&proposed_path);

            let tracked_media_conflict = tracked_media_file.as_ref().is_some_and(|media_file| {
                item.media_file_id.as_deref() != Some(media_file.id.as_str())
            });
            let tracked_collection_conflict =
                tracked_collection.as_ref().is_some_and(|collection| {
                    item.collection_id.as_deref() != Some(collection.id.as_str())
                });

            // A destination the catalog already owns is a conflict the plan can
            // report on its own. One the catalog has never seen is only visible
            // on disk, and apply refuses those in `validate_targets` rather than
            // making every previewed file pay for a stat.
            if tracked_media_conflict || tracked_collection_conflict {
                item.collision = true;
                item.reason_code = "collision_existing_tracked".into();
                item.write_action = RenameWriteAction::Error;
            }

            out.push(item);
        }

        Ok(out)
    }

    async fn read_folder_template(&self, facet_settings: RenameFacetSettings) -> AppResult<String> {
        let default_template = match facet_settings.scope_id {
            "movie" => DEFAULT_FOLDER_TEMPLATE_MOVIE,
            "series" => DEFAULT_FOLDER_TEMPLATE_SERIES,
            "anime" => DEFAULT_FOLDER_TEMPLATE_ANIME,
            _ => DEFAULT_FOLDER_TEMPLATE_MOVIE,
        };
        Ok(normalize_title_folder_template_or_default(
            self.read_setting_string_value(FOLDER_TEMPLATE_KEY, Some(facet_settings.scope_id))
                .await?,
            default_template,
            facet_settings.scope_id,
        ))
    }

    async fn read_collision_policy(
        &self,
        facet_settings: RenameFacetSettings,
    ) -> AppResult<RenameCollisionPolicy> {
        self.read_rename_policy(
            facet_settings,
            RENAME_COLLISION_POLICY_KEY,
            RENAME_COLLISION_POLICY_GLOBAL_KEY,
            facet_settings.collision_policy_key,
            parse_collision_policy,
            DEFAULT_COLLISION_POLICY,
        )
        .await
    }

    async fn read_missing_metadata_policy(
        &self,
        facet_settings: RenameFacetSettings,
    ) -> AppResult<RenameMissingMetadataPolicy> {
        self.read_rename_policy(
            facet_settings,
            RENAME_MISSING_METADATA_POLICY_KEY,
            RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY,
            facet_settings.missing_metadata_policy_key,
            parse_missing_metadata_policy,
            DEFAULT_MISSING_METADATA_POLICY,
        )
        .await
    }

    async fn read_rename_policy<T>(
        &self,
        facet_settings: RenameFacetSettings,
        scoped_key: &str,
        global_key: &str,
        handler_key: &str,
        parse: impl Fn(&str) -> Option<T>,
        default: T,
    ) -> AppResult<T> {
        let scoped = self
            .read_setting_string_value(scoped_key, Some(facet_settings.scope_id))
            .await?;
        if let Some(value) = scoped
            && let Some(parsed) = parse(&value)
        {
            return Ok(parsed);
        }

        let global = self.read_setting_string_value(global_key, None).await?;
        if let Some(value) = global
            && let Some(parsed) = parse(&value)
        {
            return Ok(parsed);
        }

        let handler_value = self.read_setting_string_value(handler_key, None).await?;
        if let Some(value) = handler_value
            && let Some(parsed) = parse(&value)
        {
            return Ok(parsed);
        }

        Ok(default)
    }
}

fn parse_collision_policy(raw: &str) -> Option<RenameCollisionPolicy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "skip" => Some(RenameCollisionPolicy::Skip),
        "error" => Some(RenameCollisionPolicy::Error),
        "replace_if_better" => Some(RenameCollisionPolicy::ReplaceIfBetter),
        _ => None,
    }
}

fn parse_missing_metadata_policy(raw: &str) -> Option<RenameMissingMetadataPolicy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "skip" => Some(RenameMissingMetadataPolicy::Skip),
        "fallback_title" => Some(RenameMissingMetadataPolicy::FallbackTitle),
        _ => None,
    }
}

fn build_movie_rename_plan_items(
    title: &Title,
    mut collections: Vec<Collection>,
    media_files: Vec<TitleMediaFile>,
    options: &mut MovieRenamePlanOptions<'_>,
) -> Vec<RenamePlanItem> {
    collections.sort_by(|left, right| left.id.cmp(&right.id));
    let media_files_by_path = media_files.into_iter().fold(
        HashMap::<String, TitleMediaFile>::new(),
        |mut acc, media_file| {
            acc.entry(media_file.file_path.clone())
                .or_insert(media_file);
            acc
        },
    );

    collections
        .into_iter()
        .map(|collection| {
            let matched_media_file = collection
                .ordered_path
                .as_deref()
                .and_then(|path| media_files_by_path.get(path));
            let mut item =
                build_movie_rename_plan_item(title, &collection, matched_media_file, options);
            if item.media_file_id.is_none() {
                item.media_file_id = matched_media_file.map(|media_file| media_file.id.clone());
            }
            item
        })
        .collect()
}

struct MovieRenamePlanOptions<'a> {
    media_root: &'a str,
    folder_template: &'a str,
    template: &'a str,
    missing_metadata_policy: &'a RenameMissingMetadataPolicy,
    planning: &'a mut RenamePlanningState,
}

const RENAME_LITERAL_PIPE_SENTINEL: char = '\u{E000}';
const RENAME_LITERAL_COLON_SENTINEL: char = '\u{E001}';

enum RenameTemplateOptionalGroupParseError {
    UnmatchedOpen,
    InvalidGuard,
    NestedOptionalGroup,
    UnsupportedFallback,
}

struct RenameTemplateOptionalGroup {
    guard: String,
    body: String,
    end_index: usize,
}

fn parse_rename_template_optional_group(
    chars: &[char],
    start_index: usize,
) -> Result<RenameTemplateOptionalGroup, RenameTemplateOptionalGroupParseError> {
    let mut cursor = start_index + 2;
    let guard_start = cursor;
    while cursor < chars.len() && chars[cursor] != ':' {
        if matches!(chars[cursor], '{' | '}') {
            return Err(RenameTemplateOptionalGroupParseError::InvalidGuard);
        }
        cursor += 1;
    }
    if cursor == chars.len() {
        return Err(RenameTemplateOptionalGroupParseError::UnmatchedOpen);
    }

    let guard_spec: String = chars[guard_start..cursor].iter().collect();
    let Some(parsed_guard) = parse_rename_template_token_spec(guard_spec.trim()) else {
        return Err(RenameTemplateOptionalGroupParseError::InvalidGuard);
    };
    if parsed_guard.pad_width.is_some() || !parsed_guard.filters.is_empty() {
        return Err(RenameTemplateOptionalGroupParseError::InvalidGuard);
    }

    let body_start = cursor + 1;
    cursor = body_start;
    let mut escaped_literal_open_count = 0usize;
    while cursor < chars.len() {
        match chars[cursor] {
            '{' if chars.get(cursor + 1).is_some_and(|next| *next == '{') => {
                escaped_literal_open_count += 1;
                cursor += 2;
            }
            '{' if chars.get(cursor + 1).is_some_and(|next| *next == '?') => {
                return Err(RenameTemplateOptionalGroupParseError::NestedOptionalGroup);
            }
            '{' => {
                let Some(end) = chars[cursor + 1..].iter().position(|value| *value == '}') else {
                    return Err(RenameTemplateOptionalGroupParseError::UnmatchedOpen);
                };
                let end_index = cursor + 1 + end;
                if chars[cursor + 1..end_index].contains(&'{') {
                    return Err(RenameTemplateOptionalGroupParseError::UnmatchedOpen);
                }
                cursor = end_index + 1;
            }
            '}' if escaped_literal_open_count > 0 => {
                if chars.get(cursor + 1).is_some_and(|next| *next == '}') {
                    cursor += 2;
                } else {
                    cursor += 1;
                }
                escaped_literal_open_count -= 1;
            }
            '|' if escaped_literal_open_count == 0
                && (chars[cursor + 1..].starts_with(&['e', 'l', 's', 'e', ':'])
                    || chars.get(cursor + 1).is_some_and(|next| *next == '?')) =>
            {
                return Err(RenameTemplateOptionalGroupParseError::UnsupportedFallback);
            }
            '}' => {
                let body: String = chars[body_start..cursor].iter().collect();
                return Ok(RenameTemplateOptionalGroup {
                    guard: parsed_guard.name,
                    body,
                    end_index: cursor,
                });
            }
            _ => cursor += 1,
        }
    }

    Err(RenameTemplateOptionalGroupParseError::UnmatchedOpen)
}

fn render_rename_template_tokens(template: &str, tokens: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut cursor = 0usize;
    let mut escaped_literal_open_count = 0usize;

    while cursor < chars.len() {
        let ch = chars[cursor];
        if ch == '{' {
            if chars.get(cursor + 1).is_some_and(|next| *next == '{') {
                out.push('{');
                escaped_literal_open_count += 1;
                cursor += 2;
                continue;
            }

            if chars.get(cursor + 1).is_some_and(|next| *next == '?')
                && let Ok(group) = parse_rename_template_optional_group(&chars, cursor)
            {
                if tokens
                    .get(&group.guard)
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    out.push_str(&render_rename_template_tokens(&group.body, tokens));
                }
                cursor = group.end_index + 1;
                continue;
            }

            if let Some(end) = chars[cursor + 1..].iter().position(|c| *c == '}') {
                let end_index = cursor + 1 + end;
                let token_spec: String = chars[cursor + 1..end_index].iter().collect();
                out.push_str(&resolve_template_token(tokens, token_spec.trim()));
                cursor = end_index + 1;
                continue;
            }
        } else if ch == '}' {
            if chars.get(cursor + 1).is_some_and(|next| *next == '}') {
                out.push('}');
                escaped_literal_open_count = escaped_literal_open_count.saturating_sub(1);
                cursor += 2;
                continue;
            }
            if escaped_literal_open_count > 0 {
                out.push('}');
                escaped_literal_open_count -= 1;
                cursor += 1;
                continue;
            }
        }

        push_rename_literal_text_char(&mut out, ch, escaped_literal_open_count);
        cursor += 1;
    }

    out
}

pub fn render_rename_template(template: &str, tokens: &BTreeMap<String, String>) -> String {
    finalize_generated_filename_component(&restore_rename_literal_sentinels(
        &render_rename_template_tokens(template, tokens),
    ))
}

fn push_rename_literal_text_char(out: &mut String, ch: char, escaped_literal_open_count: usize) {
    if escaped_literal_open_count == 0 {
        out.push(ch);
        return;
    }

    match ch {
        '|' => out.push(RENAME_LITERAL_PIPE_SENTINEL),
        ':' => out.push(RENAME_LITERAL_COLON_SENTINEL),
        _ => out.push(ch),
    }
}

fn restore_rename_literal_sentinels(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            RENAME_LITERAL_PIPE_SENTINEL => '|',
            RENAME_LITERAL_COLON_SENTINEL => ':',
            _ => ch,
        })
        .collect()
}

fn finalize_generated_filename_component(value: &str) -> String {
    truncate_generated_filename_component(&sanitize_filesystem_component(value))
}

pub fn render_title_folder_template(template: &str, tokens: &BTreeMap<String, String>) -> String {
    let raw = restore_rename_literal_sentinels(&render_rename_template_tokens(template, tokens));
    let cleaned = strip_empty_folder_template_groups(&raw);
    truncate_generated_folder_component(&sanitize_filesystem_component(cleaned.trim()))
}

pub(crate) fn validate_title_folder_template(template: &str) -> AppResult<()> {
    validate_folder_component_template(
        template,
        "folder",
        is_supported_title_folder_token,
        None,
        true,
    )
}

pub(crate) fn validate_season_folder_template(template: &str) -> AppResult<()> {
    validate_folder_component_template(
        template,
        "season folder",
        is_supported_season_folder_token,
        Some("season"),
        false,
    )
}

pub(crate) fn validate_specials_folder_template(template: &str) -> AppResult<()> {
    validate_folder_component_template(
        template,
        "specials folder",
        is_supported_season_folder_token,
        None,
        false,
    )
}

fn is_illegal_folder_template_literal(ch: char) -> bool {
    matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control()
}

#[derive(Default)]
struct RenameTemplateValidationState {
    saw_token: bool,
    saw_required_token: bool,
}

fn invalid_optional_group_error(
    template_label: &str,
    error: RenameTemplateOptionalGroupParseError,
) -> AppError {
    let message = match error {
        RenameTemplateOptionalGroupParseError::UnmatchedOpen => {
            format!("{template_label} template contains an unmatched '{{'")
        }
        RenameTemplateOptionalGroupParseError::InvalidGuard => {
            format!("{template_label} template contains an invalid optional group")
        }
        RenameTemplateOptionalGroupParseError::NestedOptionalGroup => {
            format!("{template_label} template does not support nested optional groups")
        }
        RenameTemplateOptionalGroupParseError::UnsupportedFallback => {
            format!("{template_label} template does not support optional-group fallback branches")
        }
    };
    AppError::Validation(message)
}

fn validate_rename_template_fragment<F>(
    template: &str,
    template_label: &str,
    is_supported_token: &F,
    is_illegal_literal: Option<fn(char) -> bool>,
    required_token: Option<&str>,
    state: &mut RenameTemplateValidationState,
) -> AppResult<()>
where
    F: Fn(&str) -> bool,
{
    let chars: Vec<char> = template.chars().collect();
    let mut cursor = 0usize;
    let mut escaped_literal_open_count = 0usize;

    while cursor < chars.len() {
        let ch = chars[cursor];
        if ch == '{' && chars.get(cursor + 1).is_some_and(|next| *next == '{') {
            escaped_literal_open_count += 1;
            cursor += 2;
            continue;
        }
        if ch == '}' {
            if chars.get(cursor + 1).is_some_and(|next| *next == '}') {
                escaped_literal_open_count = escaped_literal_open_count.saturating_sub(1);
                cursor += 2;
                continue;
            }
            if escaped_literal_open_count > 0 {
                escaped_literal_open_count -= 1;
                cursor += 1;
                continue;
            }
            return Err(AppError::Validation(format!(
                "{template_label} template contains an unmatched '}}'"
            )));
        }
        if ch != '{' {
            if is_illegal_literal.is_some_and(|is_illegal| is_illegal(ch)) {
                return Err(AppError::Validation(format!(
                    "{template_label} template contains an illegal filesystem character: {ch:?}"
                )));
            }
            cursor += 1;
            continue;
        }

        if chars.get(cursor + 1).is_some_and(|next| *next == '?') {
            let group = parse_rename_template_optional_group(&chars, cursor)
                .map_err(|error| invalid_optional_group_error(template_label, error))?;
            if !is_supported_token(&group.guard) {
                return Err(AppError::Validation(format!(
                    "unsupported {template_label} template token: {{{}}}",
                    group.guard
                )));
            }
            state.saw_token = true;
            if required_token.is_some_and(|required| group.guard == required) {
                state.saw_required_token = true;
            }
            validate_rename_template_fragment(
                &group.body,
                template_label,
                is_supported_token,
                is_illegal_literal,
                required_token,
                state,
            )?;
            cursor = group.end_index + 1;
            continue;
        }

        let Some(end) = chars[cursor + 1..].iter().position(|value| *value == '}') else {
            return Err(AppError::Validation(format!(
                "{template_label} template contains an unmatched '{{'"
            )));
        };
        let end_index = cursor + 1 + end;
        let token_spec: String = chars[cursor + 1..end_index].iter().collect();
        if token_spec.contains('{') {
            return Err(AppError::Validation(format!(
                "{template_label} template contains an unmatched '{{'"
            )));
        }
        let Some(parsed_token) = parse_rename_template_token_spec(token_spec.trim()) else {
            return Err(AppError::Validation(format!(
                "unsupported {template_label} template token: {{{}}}",
                token_spec.trim()
            )));
        };
        if !is_supported_token(&parsed_token.name) {
            return Err(AppError::Validation(format!(
                "unsupported {template_label} template token: {{{}}}",
                token_spec.trim()
            )));
        }
        state.saw_token = true;
        if required_token.is_some_and(|required| parsed_token.name == required) {
            state.saw_required_token = true;
        }
        cursor = end_index + 1;
    }

    Ok(())
}

fn validate_folder_component_template(
    template: &str,
    template_label: &str,
    is_supported_token: impl Fn(&str) -> bool,
    required_token: Option<&str>,
    require_any_token: bool,
) -> AppResult<()> {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(format!(
            "{template_label} template is required"
        )));
    }

    let mut state = RenameTemplateValidationState {
        saw_token: false,
        saw_required_token: required_token.is_none(),
    };
    validate_rename_template_fragment(
        trimmed,
        template_label,
        &is_supported_token,
        Some(is_illegal_folder_template_literal),
        required_token,
        &mut state,
    )?;

    if require_any_token && !state.saw_token {
        return Err(AppError::Validation(format!(
            "{template_label} template must include at least one supported token"
        )));
    }
    if !state.saw_required_token {
        return Err(AppError::Validation(format!(
            "{template_label} template must include {{{}}}",
            required_token.unwrap_or_default()
        )));
    }

    Ok(())
}

#[cfg(test)]
pub(crate) fn validate_rename_template(template: &str) -> AppResult<()> {
    validate_rename_template_with_token_checker(template, is_supported_rename_template_token)
}

pub(crate) fn validate_rename_template_for_facet(
    template: &str,
    facet: &MediaFacet,
) -> AppResult<()> {
    validate_rename_template_with_token_checker(template, |token| {
        is_supported_rename_template_token_for_facet(token, facet)
    })
}

fn validate_rename_template_with_token_checker(
    template: &str,
    is_supported_token: impl Fn(&str) -> bool,
) -> AppResult<()> {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(
            "rename template is required".to_string(),
        ));
    }

    validate_rename_template_fragment(
        trimmed,
        "rename",
        &is_supported_token,
        None,
        None,
        &mut RenameTemplateValidationState::default(),
    )?;

    Ok(())
}

pub(crate) fn normalize_title_folder_template_or_default(
    raw: Option<String>,
    default_template: &str,
    scope: &str,
) -> String {
    let Some(template) = raw
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return default_template.to_string();
    };

    match validate_title_folder_template(&template) {
        Ok(()) => template,
        Err(error) => {
            warn!(
                error = %error,
                scope,
                template = %template,
                "invalid stored folder template; using default"
            );
            default_template.to_string()
        }
    }
}

pub(crate) fn normalize_season_folder_template_or_default(raw: Option<String>) -> String {
    normalize_episode_folder_template_or_default(
        raw,
        DEFAULT_SEASON_FOLDER_TEMPLATE,
        validate_season_folder_template,
        "season",
    )
}

pub(crate) fn normalize_specials_folder_template_or_default(raw: Option<String>) -> String {
    normalize_episode_folder_template_or_default(
        raw,
        DEFAULT_SPECIALS_FOLDER_TEMPLATE,
        validate_specials_folder_template,
        "specials",
    )
}

fn normalize_episode_folder_template_or_default(
    raw: Option<String>,
    default_template: &str,
    validate: impl Fn(&str) -> AppResult<()>,
    template_kind: &str,
) -> String {
    let Some(template) = raw
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return default_template.to_string();
    };

    match validate(&template) {
        Ok(()) => template,
        Err(error) => {
            warn!(
                error = %error,
                template = %template,
                template_kind,
                "invalid stored episode folder template; using default"
            );
            default_template.to_string()
        }
    }
}

fn strip_empty_folder_template_groups(raw: &str) -> String {
    let mut cleaned = raw.to_string();
    loop {
        let updated = cleaned
            .replace("()", " ")
            .replace("[]", " ")
            .replace("{}", " ");
        if updated == cleaned {
            return updated;
        }
        cleaned = updated;
    }
}

pub(crate) fn build_title_folder_tokens(
    title: &Title,
    _parsed_year: Option<i32>,
) -> BTreeMap<String, String> {
    let (title_token, title_year_hint) = split_title_and_year_hint(&title.name);
    let resolved_year = title_year_hint
        .or_else(|| title.year.map(|value| value.to_string()))
        .unwrap_or_default();
    let mut tokens = BTreeMap::from([
        ("title".to_string(), title_token),
        ("year".to_string(), resolved_year),
    ]);
    insert_title_external_id_tokens(&mut tokens, title);
    tokens
}

pub(crate) fn render_episode_folder_name(
    title: &Title,
    season: u32,
    season_template: &str,
    specials_template: &str,
) -> String {
    let (template, default_template) = if season == 0 {
        (specials_template, DEFAULT_SPECIALS_FOLDER_TEMPLATE)
    } else {
        (season_template, DEFAULT_SEASON_FOLDER_TEMPLATE)
    };
    let mut tokens = build_title_folder_tokens(title, title.year);
    tokens.insert("season".to_string(), season.to_string());
    let rendered = render_title_folder_template(template, &tokens);
    if rendered.is_empty() {
        render_title_folder_template(default_template, &tokens)
    } else {
        rendered
    }
}

pub(crate) fn configured_title_folder_path(
    media_root: &str,
    title: &Title,
    folder_template: &str,
    parsed_year: Option<i32>,
) -> PathBuf {
    let tokens = build_title_folder_tokens(title, parsed_year);
    let folder_name = render_title_folder_template(folder_template, &tokens);
    PathBuf::from(media_root).join(if folder_name.is_empty() {
        tokens
            .get("title")
            .map(|value| sanitize_filesystem_component(value))
            .map(|value| truncate_generated_folder_component(&value))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "untitled".to_string())
    } else {
        folder_name
    })
}

pub(crate) fn effective_title_folder_path(
    media_root: &str,
    title: &Title,
    folder_template: &str,
    parsed_year: Option<i32>,
) -> PathBuf {
    title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(stored_path_to_path_buf)
        .unwrap_or_else(|| {
            configured_title_folder_path(media_root, title, folder_template, parsed_year)
        })
}

pub(crate) fn title_folder_path_for_renamed_file(
    title: &Title,
    current_file: &Path,
    media_root: &str,
    folder_template: &str,
) -> PathBuf {
    let desired_root = configured_title_folder_path(media_root, title, folder_template, title.year);
    let current_parent = current_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let Some(existing_root) = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(stored_path_to_path_buf)
    else {
        return desired_root;
    };
    let Ok(relative_parent) = current_parent.strip_prefix(&existing_root) else {
        return desired_root;
    };
    if relative_parent.as_os_str().is_empty() {
        desired_root
    } else {
        desired_root.join(relative_parent)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "episode rename placement needs its title, resolved layout, and template inputs explicitly"
)]
fn episode_parent_path_for_renamed_file(
    title: &Title,
    use_season_folders: bool,
    current_file: &Path,
    media_root: &str,
    folder_template: &str,
    season: Option<u32>,
    season_folder_template: &str,
    specials_folder_template: &str,
) -> PathBuf {
    let desired_root = configured_title_folder_path(media_root, title, folder_template, title.year);
    if !use_season_folders {
        return desired_root;
    }
    let Some(season) = season else {
        return title_folder_path_for_renamed_file(
            title,
            current_file,
            media_root,
            folder_template,
        );
    };
    desired_root.join(render_episode_folder_name(
        title,
        season,
        season_folder_template,
        specials_folder_template,
    ))
}

fn infer_title_folder_path_after_rename(
    title: &Title,
    use_season_folders: bool,
    current_path: &str,
    final_path: &str,
) -> Option<String> {
    let final_path = stored_path_to_path_buf(final_path);
    let final_parent = final_path.parent()?;
    if let Some(existing_root) = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(stored_path_to_path_buf)
    {
        let current_path = stored_path_to_path_buf(current_path);
        let current_parent = current_path.parent()?;
        if let Ok(relative_parent) = current_parent.strip_prefix(&existing_root) {
            let mut new_root = final_parent.to_path_buf();
            for _ in relative_parent.components() {
                new_root = new_root.parent()?.to_path_buf();
            }
            return Some(path_to_stored_string(&new_root));
        }
    }

    infer_title_folder_path_from_final_path(title, use_season_folders, final_parent)
        .map(|path| path_to_stored_string(&path))
}

fn infer_title_folder_path_from_final_path(
    _title: &Title,
    use_season_folders: bool,
    final_parent: &Path,
) -> Option<PathBuf> {
    if use_season_folders
        && final_parent
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("Season "))
    {
        return final_parent.parent().map(Path::to_path_buf);
    }

    Some(final_parent.to_path_buf())
}

/// Plan-wide state shared by every title in one preview.
///
/// Planning is deliberately database-only: the catalog already knows every
/// tracked file's path and size, and libraries commonly sit on network mounts
/// where a per-file stat costs a round trip. Physical truth is established at
/// apply time instead, where `validate_targets` stats each source and refuses
/// a destination that already exists before anything moves.
#[derive(Debug, Default)]
pub(crate) struct RenamePlanningState {
    planned_targets: HashSet<String>,
}

impl RenamePlanningState {
    /// Claims `key` as a plan target, returning false when it is already taken.
    fn claim_target(&mut self, key: String) -> bool {
        self.planned_targets.insert(key)
    }
}

pub fn build_rename_plan_fingerprint(
    items: &[RenamePlanItem],
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
) -> String {
    let bytes = serde_json::to_vec(&(
        template,
        collision_policy.as_str(),
        missing_metadata_policy.as_str(),
        items,
    ))
    .unwrap_or_default();
    crate::helpers::blake3_identity_hex(
        crate::helpers::HashDomain::RenamePlan,
        String::from_utf8_lossy(&bytes),
    )
}

struct GroupedTitleMediaFile {
    file: TitleMediaFile,
    episode_ids: Vec<String>,
}

struct ResolvedSeriesRenameMetadata {
    collection_id: Option<String>,
    season: String,
    season_order: String,
    episode: String,
    absolute_episode: String,
    episode_title: String,
}

#[derive(Clone)]
struct RenamePlanItemIds {
    collection_id: Option<String>,
    media_file_id: Option<String>,
    series_movie_link_ids: Vec<String>,
}

struct RenamePlanSource {
    current_path: String,
    current_file: PathBuf,
    extension: String,
    source_size_bytes: Option<u64>,
    source_mtime_unix_ms: Option<i64>,
}

struct RenameCommonTokens {
    title: String,
    year: String,
    quality: String,
    source: String,
    video_codec: String,
    audio_codec: String,
    audio_channels: String,
    group: String,
    extension: String,
}

struct ResolvedRenameCommonMetadata {
    common: RenameCommonTokens,
    edition: String,
}

impl RenamePlanSource {
    fn build_item(
        &self,
        item_ids: RenamePlanItemIds,
        proposed_path: Option<String>,
        normalized_filename: Option<String>,
        collision: bool,
        reason_code: &'static str,
        write_action: RenameWriteAction,
    ) -> RenamePlanItem {
        rename_plan_item(
            item_ids,
            self.current_path.clone(),
            proposed_path,
            normalized_filename,
            collision,
            reason_code,
            write_action,
            self.source_size_bytes,
            self.source_mtime_unix_ms,
        )
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "rename plan items mirror the persisted planning record fields explicitly"
)]
fn rename_plan_item(
    item_ids: RenamePlanItemIds,
    current_path: String,
    proposed_path: Option<String>,
    normalized_filename: Option<String>,
    collision: bool,
    reason_code: &'static str,
    write_action: RenameWriteAction,
    source_size_bytes: Option<u64>,
    source_mtime_unix_ms: Option<i64>,
) -> RenamePlanItem {
    RenamePlanItem {
        collection_id: item_ids.collection_id,
        media_file_id: item_ids.media_file_id,
        series_movie_link_ids: item_ids.series_movie_link_ids,
        current_path,
        proposed_path,
        normalized_filename,
        collision,
        reason_code: reason_code.into(),
        write_action,
        source_size_bytes,
        source_mtime_unix_ms,
    }
}

fn prepare_rename_plan_source(
    item_ids: RenamePlanItemIds,
    current_path: Option<String>,
    known_size_bytes: Option<u64>,
) -> Result<RenamePlanSource, Box<RenamePlanItem>> {
    let current_path = current_path.unwrap_or_default();
    if current_path.trim().is_empty() {
        return Err(Box::new(rename_plan_item(
            item_ids,
            current_path,
            None,
            None,
            false,
            "no_source_path",
            RenameWriteAction::Skip,
            None,
            None,
        )));
    }

    let current_file = stored_path_to_path_buf(&current_path);
    // Size comes from the catalog row; planning never stats the library.
    let source_size_bytes = known_size_bytes;
    let source_mtime_unix_ms = None;
    let source = RenamePlanSource {
        extension: current_file
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_default(),
        current_path,
        current_file,
        source_size_bytes,
        source_mtime_unix_ms,
    };

    Ok(source)
}

fn insert_common_rename_tokens(tokens: &mut BTreeMap<String, String>, common: RenameCommonTokens) {
    tokens.insert("title".to_string(), common.title);
    tokens.insert("year".to_string(), common.year);
    tokens.insert("quality".to_string(), common.quality);
    tokens.insert("source".to_string(), common.source);
    tokens.insert("video_codec".to_string(), common.video_codec);
    tokens.insert("audio_codec".to_string(), common.audio_codec);
    tokens.insert("audio_channels".to_string(), common.audio_channels);
    tokens.insert("group".to_string(), common.group);
    tokens.insert("ext".to_string(), common.extension);
}

const TITLE_EXTERNAL_ID_TOKENS: [(&str, &str); 6] = [
    ("imdb_id", "imdb"),
    ("tmdb_id", "tmdb"),
    ("tvdb_id", "tvdb"),
    ("anidb_id", "anidb"),
    ("mal_id", "mal"),
    ("anilist_id", "anilist"),
];

#[cfg(test)]
fn is_supported_rename_template_token(token: &str) -> bool {
    matches!(
        token,
        "title"
            | "year"
            | "quality"
            | "edition"
            | "source"
            | "video_codec"
            | "audio_codec"
            | "audio_channels"
            | "group"
            | "ext"
            | "season"
            | "season_order"
            | "episode"
            | "episode_title"
            | "absolute_episode"
    ) || TITLE_EXTERNAL_ID_TOKENS
        .iter()
        .any(|(token_name, _)| *token_name == token)
}

fn is_supported_rename_template_token_for_facet(token: &str, facet: &MediaFacet) -> bool {
    let common = matches!(
        token,
        "title"
            | "year"
            | "quality"
            | "source"
            | "video_codec"
            | "audio_codec"
            | "audio_channels"
            | "group"
            | "ext"
    ) || TITLE_EXTERNAL_ID_TOKENS
        .iter()
        .any(|(token_name, _)| *token_name == token);

    common
        || match facet {
            MediaFacet::Movie => token == "edition",
            MediaFacet::Series | MediaFacet::Anime => matches!(
                token,
                "season" | "season_order" | "episode" | "episode_title" | "absolute_episode"
            ),
        }
}

fn is_supported_title_folder_token(token: &str) -> bool {
    matches!(token, "title" | "year")
        || TITLE_EXTERNAL_ID_TOKENS
            .iter()
            .any(|(token_name, _)| *token_name == token)
}

fn is_supported_season_folder_token(token: &str) -> bool {
    token == "season" || is_supported_title_folder_token(token)
}

fn insert_title_external_id_tokens(tokens: &mut BTreeMap<String, String>, title: &Title) {
    for (token_name, source) in TITLE_EXTERNAL_ID_TOKENS {
        let value = if source == "imdb" {
            imdb_id_from_title(title)
        } else {
            title_external_id_value(title, source)
        }
        .unwrap_or_default();
        tokens.insert(token_name.to_string(), value);
    }
}

fn imdb_id_from_title(title: &Title) -> Option<String> {
    title
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
        .or_else(|| {
            title_external_id_value(title, "imdb")
                .and_then(|id| crate::normalize::normalize_imdb_id(&id))
        })
}

fn title_external_id_value(title: &Title, source: &str) -> Option<String> {
    title
        .external_ids
        .iter()
        .find(|id| id.source.eq_ignore_ascii_case(source))
        .map(|id| id.value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolved_analysis_labels_for_media_file(
    media_file: &TitleMediaFile,
) -> crate::media::release_labels::ResolvedAnalysisReleaseLabels {
    resolve_release_labels_from_analysis(
        media_file.video_width,
        media_file.video_height,
        media_file.video_codec.as_ref(),
        media_file.audio_codec.as_deref(),
        media_file.audio_profile.as_deref(),
        media_file.audio_channels,
        &media_file.audio_streams,
    )
}

fn resolve_rename_common_metadata(
    media_file: Option<&TitleMediaFile>,
    parsed_current: &ParsedReleaseMetadata,
    title_token: &str,
    year_token: Option<&str>,
    extension: &str,
) -> ResolvedRenameCommonMetadata {
    let analyzed = media_file
        .map(resolved_analysis_labels_for_media_file)
        .unwrap_or_default();

    let quality = analyzed
        .quality
        .or_else(|| media_file.and_then(|file| non_empty_owned(file.quality_label.clone())))
        .or_else(|| parsed_current.quality.clone())
        .unwrap_or_default();
    let source = media_file
        .and_then(|file| non_empty_owned(file.source_type.clone()))
        .or_else(|| parsed_current.source.as_ref().map(ToString::to_string))
        .unwrap_or_default();
    let video_codec = analyzed
        .video_codec
        .map(|codec| codec.to_string())
        .or_else(|| {
            media_file.and_then(|file| file.video_codec_parsed.map(|codec| codec.to_string()))
        })
        .or_else(|| parsed_current.video_codec.map(|codec| codec.to_string()))
        .unwrap_or_default();
    let audio_codec = analyzed
        .audio_codec
        .or_else(|| media_file.and_then(|file| non_empty_owned(file.audio_codec_parsed.clone())))
        .or_else(|| parsed_current.audio.as_ref().map(ToString::to_string))
        .unwrap_or_default();
    let audio_channels = analyzed
        .audio_channels
        .or_else(|| media_file.and_then(|file| non_empty_owned(file.audio_channels_parsed.clone())))
        .or_else(|| parsed_current.audio_channels.clone())
        .unwrap_or_default();
    let group = media_file
        .and_then(|file| non_empty_owned(file.release_group.clone()))
        .or_else(|| parsed_current.release_group.clone())
        .unwrap_or_default();
    let edition = media_file
        .and_then(|file| non_empty_owned(file.edition.clone()))
        .or_else(|| {
            parsed_current
                .parse_hints
                .iter()
                .find(|hint| hint.to_ascii_lowercase().contains("edition"))
                .cloned()
        })
        .unwrap_or_default();

    ResolvedRenameCommonMetadata {
        common: RenameCommonTokens {
            title: title_token.to_string(),
            year: year_token.unwrap_or_default().to_string(),
            quality,
            source,
            video_codec,
            audio_codec,
            audio_channels,
            group,
            extension: extension.to_string(),
        },
        edition,
    }
}

fn resolve_rendered_rename_filename(
    source: &RenamePlanSource,
    item_ids: RenamePlanItemIds,
    template: &str,
    tokens: &BTreeMap<String, String>,
    fallback_title: &str,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
) -> Result<String, Box<RenamePlanItem>> {
    let mut rendered = render_rename_template(template, tokens);
    if rendered.is_empty() {
        if matches!(missing_metadata_policy, RenameMissingMetadataPolicy::Skip) {
            return Err(Box::new(source.build_item(
                item_ids,
                None,
                None,
                false,
                "missing_metadata",
                RenameWriteAction::Skip,
            )));
        }
        rendered = fallback_title.to_string();
    }

    if !source.extension.is_empty()
        && !rendered
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", source.extension))
    {
        rendered = format!("{rendered}.{}", source.extension);
    }

    rendered = finalize_generated_filename_component(&rendered);

    Ok(rendered)
}

fn rename_planning_path_key(stored_path: &str) -> String {
    let normalized = lexically_normalize_rename_path(&stored_path_to_path_buf(stored_path));
    let key = {
        #[cfg(windows)]
        {
            normalized
                .to_string_lossy()
                .replace('/', "\\")
                .to_lowercase()
        }
        #[cfg(not(windows))]
        {
            normalized.to_string_lossy().into_owned()
        }
    };
    // SMB hands back decomposed names for files written precomposed, so the two
    // spellings have to key the same or every accented title plans a rename
    // that changes nothing.
    crate::stored_paths::path_identity_key(&key).unwrap_or(key)
}

fn lexically_normalize_rename_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

fn finalize_rename_plan_item(
    source: &RenamePlanSource,
    item_ids: RenamePlanItemIds,
    target_parent: PathBuf,
    rendered: String,
    planning: &mut RenamePlanningState,
) -> RenamePlanItem {
    let proposed_path_str = path_to_stored_string(target_parent.join(&rendered));
    let proposed_path_key = rename_planning_path_key(&proposed_path_str);

    if crate::stored_paths::paths_match(&proposed_path_str, &source.current_path) {
        return source.build_item(
            item_ids,
            Some(proposed_path_str),
            Some(rendered),
            false,
            "same_path",
            RenameWriteAction::Noop,
        );
    }

    if !planning.claim_target(proposed_path_key.clone()) {
        return source.build_item(
            item_ids,
            Some(proposed_path_str),
            Some(rendered),
            true,
            "collision_within_plan",
            RenameWriteAction::Skip,
        );
    }

    source.build_item(
        item_ids,
        Some(proposed_path_str),
        Some(rendered),
        false,
        "rename_move",
        RenameWriteAction::Move,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "series rename planning needs the full title, template, and collision context together"
)]
pub(crate) fn build_series_rename_plan_items_from_media_files(
    title: &Title,
    use_season_folders: bool,
    mut collections: Vec<Collection>,
    episodes: Vec<Episode>,
    media_files: Vec<TitleMediaFile>,
    media_root: &str,
    folder_template: &str,
    season_folder_template: &str,
    specials_folder_template: &str,
    template: &str,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planning: &mut RenamePlanningState,
) -> Vec<RenamePlanItem> {
    collections.sort_by(|left, right| left.id.cmp(&right.id));

    let collections_by_id = collections
        .iter()
        .cloned()
        .map(|collection| (collection.id.clone(), collection))
        .collect::<HashMap<_, _>>();
    let episodes_by_id = episodes
        .into_iter()
        .map(|episode| (episode.id.clone(), episode))
        .collect::<HashMap<_, _>>();

    let mut grouped_files = group_title_media_files(media_files);
    grouped_files.sort_by(|left, right| {
        left.file
            .file_path
            .cmp(&right.file.file_path)
            .then_with(|| left.file.id.cmp(&right.file.id))
    });

    grouped_files
        .into_iter()
        .map(|source| {
            build_series_media_file_rename_plan_item(
                title,
                use_season_folders,
                &collections,
                &collections_by_id,
                &episodes_by_id,
                source,
                media_root,
                folder_template,
                season_folder_template,
                specials_folder_template,
                template,
                missing_metadata_policy,
                planning,
            )
        })
        .collect()
}

fn group_title_media_files(media_files: Vec<TitleMediaFile>) -> Vec<GroupedTitleMediaFile> {
    let mut grouped: Vec<GroupedTitleMediaFile> = Vec::new();
    let mut indexes: HashMap<String, usize> = HashMap::new();

    for media_file in media_files {
        if let Some(index) = indexes.get(&media_file.id).copied() {
            if let Some(episode_id) = media_file.episode_id.as_ref()
                && !grouped[index]
                    .episode_ids
                    .iter()
                    .any(|value| value == episode_id)
            {
                grouped[index].episode_ids.push(episode_id.clone());
            }
            continue;
        }

        let episode_ids = media_file
            .episode_id
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        indexes.insert(media_file.id.clone(), grouped.len());
        grouped.push(GroupedTitleMediaFile {
            file: media_file,
            episode_ids,
        });
    }

    grouped
}

#[expect(
    clippy::too_many_arguments,
    reason = "single-file rename planning combines media, collection, and collision context in one decision point"
)]
fn build_series_media_file_rename_plan_item(
    title: &Title,
    use_season_folders: bool,
    collections: &[Collection],
    collections_by_id: &HashMap<String, Collection>,
    episodes_by_id: &HashMap<String, Episode>,
    source: GroupedTitleMediaFile,
    media_root: &str,
    folder_template: &str,
    season_folder_template: &str,
    specials_folder_template: &str,
    template: &str,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planning: &mut RenamePlanningState,
) -> RenamePlanItem {
    let source_item_ids = RenamePlanItemIds {
        collection_id: None,
        media_file_id: Some(source.file.id.clone()),
        series_movie_link_ids: source.file.series_movie_link_ids.clone(),
    };
    let source_file = match prepare_rename_plan_source(
        source_item_ids.clone(),
        Some(source.file.file_path.clone()),
        u64::try_from(source.file.size_bytes).ok(),
    ) {
        Ok(source_file) => source_file,
        Err(item) => return *item,
    };

    let current_stem = source_file
        .current_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(current_stem);
    let rename_metadata = resolve_series_rename_metadata(
        collections,
        collections_by_id,
        episodes_by_id,
        &source,
        &parsed,
    );
    let (title_token, year_token) = split_title_and_year_hint(&title.name);
    let fallback_year = title.year.map(|value| value.to_string());
    let extension = source_file.extension.clone();
    let common = resolve_rename_common_metadata(
        Some(&source.file),
        &parsed,
        &title_token,
        year_token.as_deref().or(fallback_year.as_deref()),
        &extension,
    );

    let mut tokens = BTreeMap::new();
    insert_common_rename_tokens(&mut tokens, common.common);
    insert_title_external_id_tokens(&mut tokens, title);
    tokens.insert("season".to_string(), rename_metadata.season.clone());
    tokens.insert(
        "season_order".to_string(),
        rename_metadata.season_order.clone(),
    );
    tokens.insert("episode".to_string(), rename_metadata.episode.clone());
    tokens.insert(
        "absolute_episode".to_string(),
        rename_metadata.absolute_episode.clone(),
    );
    tokens.insert(
        "episode_title".to_string(),
        rename_metadata.episode_title.clone(),
    );

    let item_ids = RenamePlanItemIds {
        collection_id: rename_metadata.collection_id.clone(),
        media_file_id: source_item_ids.media_file_id,
        series_movie_link_ids: source_item_ids.series_movie_link_ids,
    };
    let rendered = match resolve_rendered_rename_filename(
        &source_file,
        item_ids.clone(),
        template,
        &tokens,
        &title_token,
        missing_metadata_policy,
    ) {
        Ok(rendered) => rendered,
        Err(item) => return *item,
    };
    let target_parent = episode_parent_path_for_renamed_file(
        title,
        use_season_folders,
        &source_file.current_file,
        media_root,
        folder_template,
        rename_metadata.season.parse::<u32>().ok(),
        season_folder_template,
        specials_folder_template,
    );

    finalize_rename_plan_item(&source_file, item_ids, target_parent, rendered, planning)
}

fn resolve_series_rename_metadata(
    _collections: &[Collection],
    collections_by_id: &HashMap<String, Collection>,
    episodes_by_id: &HashMap<String, Episode>,
    source: &GroupedTitleMediaFile,
    parsed: &ParsedReleaseMetadata,
) -> ResolvedSeriesRenameMetadata {
    let linked_episodes =
        select_sorted_episodes(&source.episode_ids, episodes_by_id, collections_by_id);
    if let Some(primary_episode) = linked_episodes.first().copied() {
        let collection = primary_episode
            .collection_id
            .as_deref()
            .and_then(|collection_id| collections_by_id.get(collection_id));
        let parsed_episode = parsed.episode.as_ref();
        let season = non_empty_owned(primary_episode.season_number.clone())
            .or_else(|| collection.and_then(|value| non_empty_string(&value.collection_index)))
            .or_else(|| {
                parsed_episode
                    .and_then(|value| value.season)
                    .map(|value| value.to_string())
            })
            .unwrap_or_default();
        let episode = format_number_token(collect_episode_numbers(&linked_episodes), 2, false)
            .or_else(|| non_empty_owned(primary_episode.episode_number.clone()))
            .or_else(|| parsed_episode.and_then(parsed_episode_token))
            .unwrap_or_default();

        return ResolvedSeriesRenameMetadata {
            collection_id: None,
            season_order: collection
                .and_then(|value| non_empty_owned(value.narrative_order.clone()))
                .or_else(|| collection.and_then(|value| non_empty_string(&value.collection_index)))
                .or_else(|| non_empty_owned(primary_episode.season_number.clone()))
                .unwrap_or_else(|| season.clone()),
            absolute_episode: format_number_token(
                collect_absolute_episode_numbers(&linked_episodes),
                3,
                true,
            )
            .or_else(|| normalize_absolute_episode_token(primary_episode.absolute_number.clone()))
            .or_else(|| parsed_episode.and_then(parsed_absolute_episode_token))
            .unwrap_or_else(|| episode.clone()),
            episode_title: join_episode_titles(&linked_episodes).unwrap_or_default(),
            season,
            episode,
        };
    }

    let parsed_episode = parsed.episode.as_ref();
    let season = parsed_episode
        .and_then(|value| value.season)
        .map(|value| value.to_string())
        .unwrap_or_default();
    let episode = parsed_episode
        .and_then(parsed_episode_token)
        .unwrap_or_default();

    ResolvedSeriesRenameMetadata {
        collection_id: None,
        season_order: if season.is_empty() {
            String::new()
        } else {
            season.clone()
        },
        absolute_episode: parsed_episode
            .and_then(parsed_absolute_episode_token)
            .unwrap_or_else(|| episode.clone()),
        episode_title: String::new(),
        season,
        episode,
    }
}

fn select_sorted_episodes<'a>(
    episode_ids: &[String],
    episodes_by_id: &'a HashMap<String, Episode>,
    collections_by_id: &HashMap<String, Collection>,
) -> Vec<&'a Episode> {
    let mut episodes = episode_ids
        .iter()
        .filter_map(|episode_id| episodes_by_id.get(episode_id))
        .collect::<Vec<_>>();
    episodes.sort_by_key(|episode| episode_sort_key(episode, collections_by_id));
    episodes
}

fn collect_episode_numbers(episodes: &[&Episode]) -> Vec<u32> {
    episodes
        .iter()
        .filter_map(|episode| parse_sort_number(episode.episode_number.as_deref()))
        .collect()
}

fn collect_absolute_episode_numbers(episodes: &[&Episode]) -> Vec<u32> {
    episodes
        .iter()
        .filter_map(|episode| parse_sort_number(episode.absolute_number.as_deref()))
        .collect()
}

fn join_episode_titles(episodes: &[&Episode]) -> Option<String> {
    let mut seen = HashSet::new();
    let mut titles = Vec::new();

    for episode in episodes {
        let Some(title) = episode
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let normalized = title.to_ascii_lowercase();
        if seen.insert(normalized) {
            titles.push(title.to_string());
        }
    }

    if titles.is_empty() {
        None
    } else {
        Some(titles.join(" + "))
    }
}

fn format_number_token(mut numbers: Vec<u32>, width: usize, pad_single: bool) -> Option<String> {
    if numbers.is_empty() {
        return None;
    }

    numbers.sort_unstable();
    numbers.dedup();

    if numbers.len() == 1 {
        let value = numbers[0];
        return Some(if pad_single {
            format!("{value:0width$}")
        } else {
            value.to_string()
        });
    }

    Some(
        numbers
            .into_iter()
            .map(|value| format!("{value:0width$}"))
            .collect::<Vec<_>>()
            .join("-"),
    )
}

fn parsed_episode_token(parsed_episode: &ParsedEpisodeMetadata) -> Option<String> {
    if !parsed_episode.episode_numbers.is_empty() {
        format_number_token(parsed_episode.episode_numbers.clone(), 2, false)
    } else {
        parsed_episode
            .first_episode()
            .map(|value| value.to_string())
    }
}

fn parsed_absolute_episode_token(parsed_episode: &ParsedEpisodeMetadata) -> Option<String> {
    if !parsed_episode.absolute_episode_numbers.is_empty() {
        format_number_token(parsed_episode.absolute_episode_numbers.clone(), 3, true)
    } else {
        parsed_episode
            .absolute_episode
            .map(|value| format!("{value:03}"))
    }
}

fn episode_sort_key(
    episode: &Episode,
    collections_by_id: &HashMap<String, Collection>,
) -> (u32, u32, u32, u32, String) {
    let collection = episode
        .collection_id
        .as_deref()
        .and_then(|collection_id| collections_by_id.get(collection_id));

    (
        collection
            .and_then(|value| {
                parse_sort_number(
                    value
                        .narrative_order
                        .as_deref()
                        .or(Some(value.collection_index.as_str())),
                )
            })
            .unwrap_or(u32::MAX),
        parse_sort_number(episode.season_number.as_deref()).unwrap_or(u32::MAX),
        parse_sort_number(episode.episode_number.as_deref()).unwrap_or(u32::MAX),
        parse_sort_number(episode.absolute_number.as_deref()).unwrap_or(u32::MAX),
        episode.id.clone(),
    )
}

fn parse_sort_number(value: Option<&str>) -> Option<u32> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u32>().ok())
}

fn non_empty_owned(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn normalize_absolute_episode_token(value: Option<String>) -> Option<String> {
    non_empty_owned(value).map(|value| match value.parse::<u32>() {
        Ok(number) => format!("{number:03}"),
        Err(_) => value,
    })
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

pub(crate) fn build_rename_plan_from_items(
    facet: MediaFacet,
    title_id: Option<String>,
    template: String,
    collision_policy: RenameCollisionPolicy,
    missing_metadata_policy: RenameMissingMetadataPolicy,
    items: Vec<RenamePlanItem>,
) -> RenamePlan {
    let total = items.len();
    let renamable = items
        .iter()
        .filter(|item| matches!(item.write_action, RenameWriteAction::Move))
        .count();
    let noop = items
        .iter()
        .filter(|item| matches!(item.write_action, RenameWriteAction::Noop))
        .count();
    let conflicts = items.iter().filter(|item| item.collision).count();
    let errors = items
        .iter()
        .filter(|item| matches!(item.write_action, RenameWriteAction::Error))
        .count();

    let fingerprint = build_rename_plan_fingerprint(
        &items,
        &template,
        &collision_policy,
        &missing_metadata_policy,
    );

    RenamePlan {
        facet,
        title_id,
        template,
        collision_policy,
        missing_metadata_policy,
        fingerprint,
        total,
        renamable,
        noop,
        conflicts,
        errors,
        items,
    }
}

fn build_movie_rename_plan_item(
    title: &Title,
    collection: &Collection,
    media_file: Option<&TitleMediaFile>,
    options: &mut MovieRenamePlanOptions<'_>,
) -> RenamePlanItem {
    let item_ids = RenamePlanItemIds {
        collection_id: Some(collection.id.clone()),
        media_file_id: media_file.map(|media_file| media_file.id.clone()),
        series_movie_link_ids: Vec::new(),
    };
    let source_file = match prepare_rename_plan_source(
        item_ids.clone(),
        collection.ordered_path.clone(),
        media_file.and_then(|media_file| u64::try_from(media_file.size_bytes).ok()),
    ) {
        Ok(source_file) => source_file,
        Err(item) => return *item,
    };
    let current_stem = source_file
        .current_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(current_stem);
    let (title_token, year_token) = split_title_and_year_hint(&title.name);
    let fallback_year = title.year.map(|value| value.to_string());
    let extension = source_file.extension.clone();
    let mut common = resolve_rename_common_metadata(
        media_file,
        &parsed,
        &title_token,
        year_token.as_deref().or(fallback_year.as_deref()),
        &extension,
    );
    if common.common.quality.is_empty() {
        common.common.quality = collection
            .label
            .clone()
            .or(parsed.quality.clone())
            .unwrap_or_default();
    }

    let mut tokens = BTreeMap::new();
    let edition = common.edition.clone();
    insert_common_rename_tokens(&mut tokens, common.common);
    insert_title_external_id_tokens(&mut tokens, title);
    tokens.insert("edition".to_string(), edition);
    let rendered = match resolve_rendered_rename_filename(
        &source_file,
        item_ids.clone(),
        options.template,
        &tokens,
        &title_token,
        options.missing_metadata_policy,
    ) {
        Ok(rendered) => rendered,
        Err(item) => return *item,
    };
    let target_parent = title_folder_path_for_renamed_file(
        title,
        &source_file.current_file,
        options.media_root,
        options.folder_template,
    );

    finalize_rename_plan_item(
        &source_file,
        item_ids,
        target_parent,
        rendered,
        options.planning,
    )
}

pub(crate) fn split_title_and_year_hint(raw_title: &str) -> (String, Option<String>) {
    let trimmed = raw_title.trim();
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let Some(close_pos) = trimmed.rfind(close)
            && let Some(open_pos) = trimmed[..close_pos].rfind(open)
        {
            let candidate = trimmed[open_pos + 1..close_pos].trim();
            if candidate.len() == 4 && candidate.chars().all(|value| value.is_ascii_digit()) {
                let title = trimmed[..open_pos].trim().to_string();
                if !title.is_empty() {
                    return (title, Some(candidate.to_string()));
                }
            }
        }
    }

    (trimmed.to_string(), None)
}

enum RenameTemplateTokenFilter {
    Space(String),
    Truncate(usize),
}

struct RenameTemplateTokenSpec {
    name: String,
    pad_width: Option<usize>,
    filters: Vec<RenameTemplateTokenFilter>,
}

fn resolve_template_token(tokens: &BTreeMap<String, String>, token_spec: &str) -> String {
    let Some(spec) = parse_rename_template_token_spec(token_spec) else {
        return String::new();
    };
    let raw = tokens.get(&spec.name).cloned().unwrap_or_default();
    let mut rendered = match spec.pad_width {
        Some(width) if width > 0 => {
            if raw.chars().all(|c| c.is_ascii_digit()) && !raw.is_empty() {
                format!("{:0>width$}", raw, width = width)
            } else {
                raw
            }
        }
        _ => raw,
    };

    for filter in spec.filters {
        match filter {
            RenameTemplateTokenFilter::Space(replacement) => {
                rendered = replace_token_whitespace(&rendered, &replacement);
            }
            RenameTemplateTokenFilter::Truncate(limit) => {
                rendered = truncate_token_chars(&rendered, limit);
            }
        }
    }

    rendered
}

fn parse_rename_template_token_spec(token_spec: &str) -> Option<RenameTemplateTokenSpec> {
    let mut parts = token_spec.split('|');
    let token_core = parts.next().unwrap_or("").trim();
    if token_core.is_empty() {
        return None;
    }
    let (name, pad_width) = match token_core.split_once(':') {
        Some((n, fmt)) => {
            let fmt = fmt.trim();
            if fmt.is_empty() || !fmt.chars().all(|ch| ch.is_ascii_digit()) {
                return None;
            }
            let pad_width = fmt.parse::<usize>().ok()?;
            if pad_width > MAX_RENAME_TEMPLATE_PADDING_WIDTH {
                return None;
            }
            (n.trim().to_lowercase(), Some(pad_width))
        }
        None => (token_core.trim().to_lowercase(), None),
    };
    if name.is_empty() {
        return None;
    }
    let mut filters = Vec::new();
    for filter_spec in parts {
        filters.push(parse_rename_template_token_filter(filter_spec)?);
    }

    Some(RenameTemplateTokenSpec {
        name,
        pad_width,
        filters,
    })
}

fn parse_rename_template_token_filter(filter_spec: &str) -> Option<RenameTemplateTokenFilter> {
    let filter_spec = filter_spec.trim();
    if let Some(replacement) = filter_spec.strip_prefix("space:") {
        return match replacement {
            "_" | "." | "-" | "" => Some(RenameTemplateTokenFilter::Space(replacement.to_string())),
            _ => None,
        };
    }

    let raw_limit = filter_spec.strip_prefix("truncate:")?;
    if !raw_limit.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let limit = raw_limit.parse::<usize>().ok()?;
    (limit > 0).then_some(RenameTemplateTokenFilter::Truncate(limit))
}

fn replace_token_whitespace(value: &str, replacement: &str) -> String {
    let mut rendered = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_whitespace() {
            rendered.push_str(replacement);
        } else {
            rendered.push(ch);
        }
    }
    rendered
}

fn truncate_token_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

pub fn sanitize_filesystem_component(raw: &str) -> String {
    let mut sanitized = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch <= '\u{1f}' {
            sanitized.push(' ');
        } else {
            sanitized.push(ch);
        }
    }

    disarm_windows_reserved_device_name(&collapse_separators(&sanitized))
}

fn truncate_generated_filename_component(component: &str) -> String {
    truncate_generated_component(component, true)
}

fn truncate_generated_folder_component(component: &str) -> String {
    truncate_generated_component(component, false)
}

fn truncate_generated_component(component: &str, preserve_extension: bool) -> String {
    let budget =
        GENERATED_COMPONENT_MAX_BYTES.saturating_sub(GENERATED_COMPONENT_SUFFIX_RESERVE_BYTES);
    if component.len() <= budget {
        return component.to_string();
    }

    if preserve_extension {
        let path = Path::new(component);
        if let Some(extension) = path.extension().and_then(|value| value.to_str())
            && !extension.is_empty()
        {
            let extension_with_dot = format!(".{extension}");
            if extension_with_dot.len() < budget {
                let stem = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or(component);
                let stem_budget = budget - extension_with_dot.len();
                let stem = trim_truncated_component_end(&truncate_utf8_bytes(stem, stem_budget));
                if !stem.is_empty() {
                    return disarm_windows_reserved_device_name(&format!(
                        "{stem}{extension_with_dot}"
                    ));
                }
            }
        }
    }

    let truncated = trim_truncated_component_end(&truncate_utf8_bytes(component, budget));
    if truncated.is_empty() {
        String::new()
    } else {
        disarm_windows_reserved_device_name(&truncated)
    }
}

fn truncate_utf8_bytes(value: &str, budget: usize) -> String {
    if value.len() <= budget {
        return value.to_string();
    }

    let mut end = 0usize;
    for (index, ch) in value.char_indices() {
        let next = index + ch.len_utf8();
        if next > budget {
            break;
        }
        end = next;
    }
    value[..end].to_string()
}

fn trim_truncated_component_end(value: &str) -> String {
    value
        .trim_end_matches(|ch: char| ch.is_whitespace() || matches!(ch, '.' | '-' | '_'))
        .to_string()
}

fn disarm_windows_reserved_device_name(component: &str) -> String {
    let Some((prefix_len, suffix_start)) = windows_reserved_device_name_bounds(component) else {
        return component.to_string();
    };

    let mut disarmed = String::with_capacity(component.len() + 1);
    disarmed.push_str(&component[..prefix_len]);
    disarmed.push('_');
    disarmed.push_str(&component[suffix_start..]);
    disarmed
}

fn windows_reserved_device_name_bounds(component: &str) -> Option<(usize, usize)> {
    const RESERVED_DEVICE_NAMES: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    RESERVED_DEVICE_NAMES.iter().find_map(|reserved| {
        let prefix = component.get(..reserved.len())?;
        if !prefix.eq_ignore_ascii_case(reserved) {
            return None;
        }
        let suffix = &component[reserved.len()..];
        let Some(first) = suffix.chars().next() else {
            return Some((reserved.len(), reserved.len()));
        };
        if first == '.' {
            return Some((reserved.len(), reserved.len()));
        }
        if first == '-' || first.is_whitespace() {
            let separator_len = suffix
                .char_indices()
                .take_while(|(_, ch)| *ch == '-' || ch.is_whitespace())
                .map(|(index, ch)| index + ch.len_utf8())
                .last()
                .unwrap_or(first.len_utf8());
            return Some((reserved.len(), reserved.len() + separator_len));
        }
        None
    })
}

fn collapse_separators(raw: &str) -> String {
    let mut collapsed = String::with_capacity(raw.len());
    let mut previous: Option<char> = None;

    for ch in raw.chars() {
        let normalized = if ch.is_whitespace() { ' ' } else { ch };
        let is_separator = matches!(normalized, ' ' | '.' | '-' | '_');
        if is_separator && previous.is_some_and(|prev| prev == normalized) {
            continue;
        }
        collapsed.push(normalized);
        previous = Some(normalized);
    }

    collapsed
        .trim_matches(|value: char| value.is_whitespace() || matches!(value, '.' | '-' | '_'))
        .to_string()
}

#[cfg(test)]
#[path = "library_rename_tests.rs"]
mod library_rename_tests;

/// The library a plan's files belong to, for plans that do not name a title.
///
/// Every plan the API can produce today is scoped to one title, so this only
/// keeps the permission lookup honest if that ever changes.
async fn first_library_id_in_plan(app: &AppUseCase, preview: &RenamePlan) -> Option<String> {
    for item in &preview.items {
        let probe = RenameApplyItemResult {
            collection_id: item.collection_id.clone(),
            media_file_id: item.media_file_id.clone(),
            series_movie_link_ids: item.series_movie_link_ids.clone(),
            current_path: item.current_path.clone(),
            proposed_path: item.proposed_path.clone(),
            final_path: None,
            write_action: item.write_action.clone(),
            status: RenameApplyStatus::Skipped,
            reason_code: String::new(),
            error_message: None,
        };
        if let Some(library_id) = app.library_id_for_rename_item(&probe).await {
            return Some(library_id);
        }
    }
    None
}

// ── Rename as a background job ───────────────────────────────────────────────
//
// Renaming walks every file of every selected title and moves them on disk, so
// it does not belong on a request. It runs as a job, and each title it touches
// is locked for the duration so a second rename cannot start moving the same
// files underneath the first.

#[derive(Clone, Debug)]
pub struct RenameTitlesJobAccepted {
    pub job_run: crate::JobRun,
    pub accepted_title_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleRenameProgress {
    status: String,
    phase: String,
    total: usize,
    processed: usize,
    succeeded: usize,
    failed: usize,
    files_renamed: usize,
    current_title: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleRenameSummary {
    total: usize,
    succeeded: usize,
    failed: usize,
    files_renamed: usize,
    failures: Vec<TitleRenameFailure>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleRenameFailure {
    title_id: String,
    message: String,
}

/// The guard key reserving one title against concurrent renames.
fn title_rename_guard_key(title_id: &str) -> String {
    format!("title-rename:{title_id}")
}

impl AppUseCase {
    /// Accepts a rename for the given titles and returns immediately.
    ///
    /// Every title is authorized and locked before the job starts, so a caller
    /// either gets the whole set reserved or a clear failure naming the title
    /// that is already being renamed.
    pub async fn start_rename_titles_job(
        &self,
        actor: &User,
        title_ids: &[String],
        facet: MediaFacet,
    ) -> AppResult<RenameTitlesJobAccepted> {
        if title_ids.is_empty() {
            return Err(AppError::Validation(
                "at least one title is required for renaming".into(),
            ));
        }
        if !self.resolve_rename_enabled(&facet).await? {
            return Err(AppError::Validation("renamer_disabled".into()));
        }

        let mut seen = HashSet::new();
        let mut titles = Vec::with_capacity(title_ids.len());
        for title_id in title_ids {
            if !seen.insert(title_id.clone()) {
                return Err(AppError::Validation(format!(
                    "duplicate title id in rename request: {title_id}"
                )));
            }
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
            if title.facet != facet {
                return Err(AppError::Validation(
                    "requested facet does not match title facet".into(),
                ));
            }
            titles.push(title);
        }

        // Hold a guard per title for the life of the job. Acquiring them all up
        // front means a partially reserved set never starts moving files.
        let mut guards = Vec::with_capacity(titles.len());
        for title in &titles {
            let guard = self
                .runtime
                .jobs
                .interactive_operation_guards
                .try_acquire(&title_rename_guard_key(&title.id))
                .await
                .ok_or_else(|| {
                    AppError::Validation(format!("a rename is already running for {}", title.name))
                })?;
            guards.push(guard);
        }

        let accepted_title_ids = titles
            .iter()
            .map(|title| title.id.clone())
            .collect::<Vec<_>>();
        let now = chrono::Utc::now();
        let mut run = crate::JobRunRecord {
            id: scryer_domain::Id::new().0,
            job_key: crate::JobKey::TitleRename,
            operation_type: format!("title_rename:{}", accepted_title_ids.len()),
            status: crate::JobRunStatus::Running,
            trigger_source: crate::JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::to_string(&TitleRenameProgress {
                status: crate::JobRunStatus::Running.as_str().to_string(),
                phase: "queued".to_string(),
                total: accepted_title_ids.len(),
                processed: 0,
                succeeded: 0,
                failed: 0,
                files_renamed: 0,
                current_title: None,
            })
            .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let run_payload = crate::JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;
        let actor_event = crate::domain_events::DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(scryer_domain::JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let app = self.clone();
        let job_actor = actor.clone();
        tokio::spawn(async move {
            app.run_rename_titles_job(run, job_actor, actor_event, titles, guards)
                .await;
        });

        Ok(RenameTitlesJobAccepted {
            job_run: run_payload,
            accepted_title_ids,
        })
    }

    async fn run_rename_titles_job(
        &self,
        mut run: crate::JobRunRecord,
        actor: User,
        actor_event: crate::domain_events::DomainEventActor,
        titles: Vec<Title>,
        _guards: Vec<tokio::sync::OwnedMutexGuard<()>>,
    ) {
        let total = titles.len();
        let mut succeeded = 0usize;
        let mut files_renamed = 0usize;
        let mut failures = Vec::new();

        for (index, title) in titles.iter().enumerate() {
            let _ = self
                .update_title_rename_progress(
                    &mut run,
                    TitleRenameProgress {
                        status: crate::JobRunStatus::Running.as_str().to_string(),
                        phase: "renaming".to_string(),
                        total,
                        processed: index,
                        succeeded,
                        failed: failures.len(),
                        files_renamed,
                        current_title: Some(title.name.clone()),
                    },
                )
                .await;

            // Each title is previewed and applied inside the job so the plan is
            // built against what is on disk now, not what the caller saw.
            let outcome = async {
                let preview = self
                    .preview_rename_for_title(&actor, &title.id, title.facet.clone())
                    .await?;
                let fingerprint = preview.fingerprint.clone();
                self.apply_rename_for_title(&actor, &title.id, title.facet.clone(), &fingerprint)
                    .await
            }
            .await;

            match outcome {
                Ok(result) => {
                    succeeded += 1;
                    files_renamed += result.applied;
                }
                Err(error) => failures.push(TitleRenameFailure {
                    title_id: title.id.clone(),
                    message: error.to_string(),
                }),
            }
        }

        let failed = failures.len();
        let status = if succeeded == 0 && failed > 0 {
            crate::JobRunStatus::Failed
        } else if failed > 0 {
            crate::JobRunStatus::Warning
        } else {
            crate::JobRunStatus::Completed
        };
        let summary_text = format!(
            "Renamed {files_renamed} file(s) across {succeeded} title(s); {failed} failed."
        );
        let _ = self
            .finish_title_rename_job(
                run,
                actor_event,
                status,
                summary_text,
                TitleRenameSummary {
                    total,
                    succeeded,
                    failed,
                    files_renamed,
                    failures,
                },
            )
            .await;
    }

    async fn update_title_rename_progress(
        &self,
        run: &mut crate::JobRunRecord,
        progress: TitleRenameProgress,
    ) -> AppResult<()> {
        run.progress_json = serde_json::to_string(&progress).ok();
        run.updated_at = chrono::Utc::now();
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(crate::JobRun::from_record(&updated, None))
            .await;
        Ok(())
    }

    async fn finish_title_rename_job(
        &self,
        mut run: crate::JobRunRecord,
        actor: crate::domain_events::DomainEventActor,
        status: crate::JobRunStatus,
        summary_text: String,
        summary: TitleRenameSummary,
    ) -> AppResult<()> {
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = Some(
            serde_json::json!({
                "status": status.as_str(),
                "phase": "completed",
                "total": summary.total,
                "processed": summary.total,
                "succeeded": summary.succeeded,
                "failed": summary.failed,
                "filesRenamed": summary.files_renamed,
                "currentTitle": null,
            })
            .to_string(),
        );
        run.summary_text = Some(summary_text);
        run.summary_json = serde_json::to_string(&summary).ok();
        run.error_text =
            (status == crate::JobRunStatus::Failed).then(|| "all title renames failed".to_string());
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(crate::JobRun::from_record(&updated, None))
            .await;
        let payload = if status == crate::JobRunStatus::Failed {
            DomainEventPayload::JobRunFailed(scryer_domain::JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text: updated.error_text.clone(),
            })
        } else {
            DomainEventPayload::JobRunCompleted(scryer_domain::JobRunCompletedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                summary_text: updated.summary_text.clone(),
            })
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }
}
