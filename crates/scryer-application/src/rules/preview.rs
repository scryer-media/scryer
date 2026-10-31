//! Read-only title-aware scoring previews for the rule editor.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use scryer_domain::{AppPermission, Id, MediaFacet, RuleSet, User};
use serde_json::Value;

use crate::canonical_scoring::ListingFacts;
use crate::quality::release_listing::{
    ReleaseListingSnapshot, bounded_extra, bounded_indexer_languages,
};
use crate::{AppError, AppResult, AppUseCase};

/// The unsaved fields from the scoring-rule editor.
#[derive(Clone, Debug)]
pub struct RuleSetTestDraft {
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub enabled: bool,
    pub priority: i32,
    pub applied_facets: Vec<MediaFacet>,
}

/// A release and editor state to evaluate without storing either.
#[derive(Clone, Debug)]
pub struct RuleSetTestRequest {
    pub draft: Option<RuleSetTestDraft>,
    pub test_rule_set_id: Option<String>,
    pub edit_rule_set_id: Option<String>,
    pub copy_source_rule_set_id: Option<String>,
    pub copy_disables_source: bool,
    pub title_id: String,
    pub episode_id: Option<String>,
    /// Release name to parse and score. Exactly one of this and
    /// `media_file_id` is required.
    pub release_name: Option<String>,
    pub size_bytes: Option<i64>,
    /// Indexer listing facts for `release_name`, as a live listing would
    /// report them. Absent values stay unknown, exactly as a listing that
    /// lacks them. Not accepted with `media_file_id`.
    pub listing: RuleSetTestListingInput,
    /// Stored media file of the selected title to score from its row and its
    /// frozen listing snapshot, instead of a release name.
    pub media_file_id: Option<String>,
}

/// Indexer listing facts supplied to the tester for a release name.
#[derive(Clone, Debug, Default)]
pub struct RuleSetTestListingInput {
    pub protocol: Option<scryer_rules::ReleaseProtocol>,
    /// Publish time, in any form a live listing's is kept in: RFC 2822
    /// (a newznab `pubDate`) or RFC 3339.
    pub published_at: Option<String>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub is_password_protected: Option<bool>,
    pub indexer_languages: Option<Vec<String>>,
    /// Indexer-specific scalars; bounded the same way a live listing's are.
    pub extra: Option<serde_json::Map<String, Value>>,
}

impl RuleSetTestListingInput {
    fn is_empty(&self) -> bool {
        self.published_at.is_none()
            && self.protocol.is_none()
            && self.thumbs_up.is_none()
            && self.thumbs_down.is_none()
            && self.is_password_protected.is_none()
            && self.indexer_languages.is_none()
            && self.extra.is_none()
    }

    /// The snapshot a never-grabbed candidate with these facts would be
    /// scored with at `now`.
    fn snapshot(&self, now: DateTime<Utc>) -> ReleaseListingSnapshot {
        let extra: HashMap<String, Value> = self
            .extra
            .as_ref()
            .map(|extra| {
                extra
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        ReleaseListingSnapshot {
            protocol: self.protocol,
            grab_scoring: None,
            published_at: self
                .published_at
                .as_deref()
                .map(|raw| raw.trim().to_string()),
            thumbs_up: self.thumbs_up,
            thumbs_down: self.thumbs_down,
            is_password_protected: self.is_password_protected,
            indexer_languages: bounded_indexer_languages(self.indexer_languages.iter().flatten()),
            extra: bounded_extra(&extra),
            captured_at: now,
        }
    }
}

/// The listing facts a preview scored with, and the age rules saw.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleSetTestListingFacts {
    pub protocol: Option<scryer_rules::ReleaseProtocol>,
    pub published_at: Option<String>,
    /// `input.release.age_days`: the age at `captured_at` for a stored file,
    /// the age now for a tested release name.
    pub age_days: Option<i64>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub is_password_protected: Option<bool>,
    pub indexer_languages: Vec<String>,
    pub extra: BTreeMap<String, Value>,
    pub captured_at: DateTime<Utc>,
}

impl RuleSetTestListingFacts {
    fn from_facts(facts: &ListingFacts) -> Self {
        let snapshot = &facts.snapshot;
        Self {
            published_at: snapshot.published_at.clone(),
            protocol: snapshot.protocol,
            age_days: snapshot.age_days(facts.anchor),
            thumbs_up: snapshot.thumbs_up,
            thumbs_down: snapshot.thumbs_down,
            is_password_protected: snapshot.is_password_protected,
            indexer_languages: snapshot.indexer_languages.clone(),
            extra: snapshot.extra.clone(),
            captured_at: snapshot.captured_at,
        }
    }
}

/// What a preview scores: a release name as a fresh candidate, or a stored
/// file exactly as its landed bar is derived.
enum PreviewSubject {
    Release {
        name: String,
        size_bytes: Option<i64>,
        listing: Box<ListingFacts>,
    },
    StoredFile {
        file: Box<crate::TitleMediaFile>,
        /// Every episode the file is linked to; its size basis.
        episode_ids: Vec<String>,
        /// The episodes its bar is scored for: the selected episode, else
        /// the whole span. Empty for a file bound to no episode.
        scoring_episode_ids: Vec<String>,
    },
}

#[derive(Clone, Debug)]
struct SavedRulePreviewMetadata {
    id: String,
    name: String,
    is_managed: bool,
    enabled: bool,
    applied_facets: Vec<MediaFacet>,
}

#[derive(Clone, Debug)]
pub struct RuleSetTestContext {
    pub title_name: String,
    pub library_name: Option<String>,
    pub facet: String,
    pub language: Option<String>,
    pub tags: Vec<String>,
    pub episode_label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RuleSetTestParsed {
    pub quality: Option<String>,
    pub source: Option<String>,
    pub season: Option<String>,
    pub episode: Option<String>,
    pub edition: Option<String>,
    pub size_bytes: Option<i64>,
    pub release_group: Option<String>,
    pub video_codec: Option<String>,
    pub audio: Option<String>,
    pub year: Option<i32>,
    pub audio_languages: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RuleSetTestEntry {
    pub code: String,
    pub delta: i32,
    pub blocked: bool,
    pub kind: crate::quality_profile::ScoringEntryKind,
}

#[derive(Clone, Debug)]
pub struct RuleSetTestRuleSetResult {
    pub rule_set_id: Option<String>,
    pub rule_set_name: String,
    pub origin: String,
    pub score: i32,
    pub matched: bool,
    pub blocked: bool,
    pub is_draft: bool,
    pub messages: Vec<String>,
    pub entries: Vec<RuleSetTestEntry>,
}

#[derive(Clone, Debug)]
pub struct RuleSetTestDraftContribution {
    pub score: i32,
    pub matched: bool,
    pub blocked: bool,
    pub applies: bool,
    pub enabled: bool,
    pub message: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RuleSetTestResult {
    /// The release name that was scored: the supplied one, or a stored file's
    /// grabbed release title (its file name when it has none).
    pub release_name: String,
    /// The stored file that was scored, when testing one.
    pub media_file_id: Option<String>,
    /// Listing facts rules read. `None` for a stored file without a listing
    /// snapshot: every listing fact was unknown.
    pub listing: Option<RuleSetTestListingFacts>,
    pub score: i32,
    pub allowed: bool,
    pub blocked: bool,
    pub minimum_score_met: bool,
    pub profile_name: String,
    pub context: RuleSetTestContext,
    pub parsed: RuleSetTestParsed,
    pub rule_sets: Vec<RuleSetTestRuleSetResult>,
    pub draft_contribution: RuleSetTestDraftContribution,
    pub errors: Vec<RuleSetTestError>,
}

#[derive(Clone, Debug)]
pub struct RuleSetTestError {
    pub code: String,
    pub message: String,
    pub rule_set_id: Option<String>,
}

impl AppUseCase {
    /// Compile a one-request scoring engine with the editor draft substituted.
    /// This never writes a rule, changes the live engine, creates history, or
    /// invokes an admission/download workflow.
    pub async fn test_rule_set(
        &self,
        actor: &User,
        mut request: RuleSetTestRequest,
    ) -> AppResult<RuleSetTestResult> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        normalize_preview_subject(&mut request);
        validate_preview_request(&request)?;

        let title = self
            .get_title(actor, &request.title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::catalog::TITLE_NOT_FOUND,
                    format!("title {}", request.title_id),
                )
            })?;
        let now = self.runtime.environment.now();

        // A stored file is loaded only through the title the actor was just
        // allowed to view. It is scored for the selected episode, else over
        // its whole episode span.
        let stored_file = match request.media_file_id.as_deref() {
            Some(media_file_id) => {
                let rows: Vec<crate::TitleMediaFile> = self
                    .services
                    .library
                    .media_files
                    .list_media_files_for_title(&title.id)
                    .await?
                    .into_iter()
                    .filter(|row| row.id == media_file_id)
                    .collect();
                let mut episode_ids: Vec<String> = rows
                    .iter()
                    .filter_map(|row| row.episode_id.clone())
                    .collect();
                episode_ids.sort();
                episode_ids.dedup();
                let file = rows.into_iter().next().ok_or_else(|| {
                    AppError::not_found_reason(
                        crate::error_reasons::rules::MEDIA_FILE_NOT_FOUND,
                        format!("media file {media_file_id} of title {}", title.id),
                    )
                })?;
                if let Some(episode_id) = request.episode_id.as_deref()
                    && !episode_ids.iter().any(|id| id == episode_id)
                {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::rules::MEDIA_FILE_DOES_NOT_COVER_EPISODE,
                        format!("media file {media_file_id} does not cover episode {episode_id}"),
                    ));
                }
                Some((file, episode_ids))
            }
            None => None,
        };
        let episode_id = request.episode_id.clone().or_else(|| {
            stored_file.as_ref().and_then(|(file, episode_ids)| {
                file.episode_id
                    .clone()
                    .or_else(|| episode_ids.first().cloned())
            })
        });

        let episode = match episode_id.as_deref() {
            Some(episode_id) => {
                let episode = self
                    .services
                    .catalog
                    .shows
                    .get_episode_by_id(episode_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::not_found_reason(
                            crate::error_reasons::rules::EPISODE_NOT_FOUND,
                            format!("episode {episode_id}"),
                        )
                    })?;
                if episode.title_id != title.id {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::rules::EPISODE_DOES_NOT_BELONG_TITLE,
                        format!("episode {episode_id} does not belong to title {}", title.id),
                    ));
                }
                Some(episode)
            }
            None => None,
        };
        if title.facet == MediaFacet::Movie && episode.is_some() {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::EPISODE_CANNOT_SELECTED_WHEN_TESTING_MOVIE_RULE,
                "an episode cannot be selected when testing a movie rule",
            ));
        }
        // A stored file bound to no episode is scored over the whole file.
        if matches!(title.facet, MediaFacet::Series | MediaFacet::Anime)
            && episode.is_none()
            && stored_file.is_none()
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE,
                "an episode is required when testing a series or anime rule",
            ));
        }

        if request.test_rule_set_id.is_some()
            && (request.draft.is_some()
                || request.edit_rule_set_id.is_some()
                || request.copy_source_rule_set_id.is_some()
                || request.copy_disables_source)
        {
            return Err(AppError::validation_refused(crate::error_reasons::rules::TESTING_SAVED_RULE_CANNOT_INCLUDE_DRAFT_EDIT_COPY_CONTEXT, "testing a saved rule cannot include a draft, edit, or copy context"));
        }
        if request.test_rule_set_id.is_none() && request.draft.is_none() {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::PREVIEW_REQUIRES_EITHER_DRAFT_SAVED_RULE_SET,
                "a preview requires either a draft or a saved rule set",
            ));
        }
        if request.edit_rule_set_id.is_some() && request.copy_source_rule_set_id.is_some() {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::PREVIEW_CANNOT_EDIT_COPY_RULE_SAME_TIME,
                "a preview cannot edit and copy a rule at the same time",
            ));
        }
        if request.copy_disables_source && request.copy_source_rule_set_id.is_none() {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::COPY_DISABLES_SOURCE_REQUIRES_COPY_SOURCE_RULE_SET,
                "copy_disables_source requires a copy source rule set",
            ));
        }
        // Take one consistent policy snapshot while mutations are excluded.
        // Compilation happens after this scope, so a slow/large preview never
        // holds the mutation lock or replaces the production engine.
        let (
            rule_sets,
            plugin_policies,
            draft_phase,
            draft_exclusive_group,
            draft_tags,
            draft,
            draft_id,
            saved_rule_test,
            saved_rule_result,
        ) = {
            let _mutation = self.services.customization.rule_mutation_lock.lock().await;
            let saved_rule = match request.test_rule_set_id.as_deref() {
                Some(id) => Some(self.require_preview_source_rule(id).await?),
                None => None,
            };
            let saved_rule_result = saved_rule.as_ref().map(|rule| SavedRulePreviewMetadata {
                id: rule.id.clone(),
                name: rule.name.clone(),
                is_managed: rule.is_managed,
                enabled: rule.enabled,
                applied_facets: rule.applied_facets.clone(),
            });
            let edit_source = match request.edit_rule_set_id.as_deref() {
                Some(id) => Some(self.require_preview_source_rule(id).await?),
                None => None,
            };
            let copy_source = match request.copy_source_rule_set_id.as_deref() {
                Some(id) => Some(self.require_preview_source_rule(id).await?),
                None => None,
            };
            if let Some(source) = edit_source.as_ref() {
                let tracked = self
                    .services
                    .customization
                    .rule_sets
                    .find_rule_pack_installation_by_rule_set_id(&source.id)
                    .await?
                    .is_some();
                if source.is_managed || tracked {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::rules::MANAGED_TRACKED_RULES_INVALID,
                        "managed or tracked rules must be copied before editing",
                    ));
                }
            }
            let mut enabled = self
                .services
                .customization
                .rule_sets
                .list_enabled_rule_sets()
                .await?;
            // Edits retain their saved metadata. An ordinary copy is a new
            // user rule and follows create_rule_set's additional/default
            // metadata; only the tracked copy-and-disable path preserves the
            // managed source's phase, group, and tag scope.
            let mut tracked_copy = false;
            if let Some(source) = edit_source.as_ref() {
                enabled.retain(|rule_set| rule_set.id != source.id);
            }
            if let Some(source) = copy_source.as_ref() {
                let tracked = self
                    .services
                    .customization
                    .rule_sets
                    .find_rule_pack_installation_by_rule_set_id(&source.id)
                    .await?;
                if tracked.as_ref().is_some_and(|pack| !pack.customizable) {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::rules::RULES_PACK_CANNOT_COPIED,
                        "rules from this pack cannot be copied",
                    ));
                }
                let tracked = tracked.is_some();
                if tracked && !request.copy_disables_source {
                    return Err(AppError::validation_refused(
                        crate::error_reasons::rules::COPYING_TRACKED_RULE_REQUIRES_DISABLING_SOURCE,
                        "copying a tracked rule requires disabling its source",
                    ));
                }
                if tracked && request.copy_disables_source {
                    enabled.retain(|rule_set| rule_set.id != source.id);
                }
                tracked_copy = tracked;
            }
            let inherited_source = edit_source
                .as_ref()
                .or_else(|| tracked_copy.then_some(copy_source.as_ref()).flatten());
            let draft_phase = inherited_source
                .map(|source| source.evaluation_phase)
                .unwrap_or_default();
            let draft_exclusive_group =
                inherited_source.and_then(|source| source.exclusive_group.clone());
            let draft_tags = inherited_source.and_then(|source| source.managed_tag_filter.clone());
            let plugin_policies = self
                .services
                .integrations
                .plugin_provider
                .available()
                .map(|provider| provider.scoring_policies())
                .unwrap_or_default();
            let (draft, draft_id, saved_rule_test) = match saved_rule {
                Some(rule) => (
                    RuleSetTestDraft {
                        name: rule.name,
                        description: rule.description,
                        rego_source: rule.rego_source,
                        enabled: rule.enabled,
                        priority: rule.priority,
                        applied_facets: rule.applied_facets,
                    },
                    rule.id,
                    true,
                ),
                None => (
                    request.draft.clone().expect("validated preview draft"),
                    request
                        .edit_rule_set_id
                        .clone()
                        .unwrap_or_else(|| Id::new_rego_safe().0),
                    false,
                ),
            };
            (
                enabled,
                plugin_policies,
                draft_phase,
                draft_exclusive_group,
                draft_tags,
                draft,
                draft_id,
                saved_rule_test,
                saved_rule_result,
            )
        };

        let profile = self.resolve_quality_profile_for_title(&title).await?;
        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;
        let context = self
            .resolve_canonical_scoring_context_without_rules(&title, &profile)
            .await;
        let compute_title = title.clone();
        let compute_episode = episode.clone();
        let compute_draft = draft.clone();
        let compute_draft_id = draft_id.clone();
        let subject = match (stored_file, request.release_name.clone()) {
            (Some((file, episode_ids)), _) => PreviewSubject::StoredFile {
                file: Box::new(file),
                scoring_episode_ids: match request.episode_id.clone() {
                    Some(episode_id) => vec![episode_id],
                    None => episode_ids.clone(),
                },
                episode_ids,
            },
            (None, Some(name)) => PreviewSubject::Release {
                name,
                size_bytes: request.size_bytes,
                // A tester candidate has never been grabbed: its age is
                // measured now, like any live search result.
                listing: Box::new(ListingFacts::candidate(request.listing.snapshot(now), now)),
            },
            (None, None) => unreachable!("validated preview subject"),
        };
        let scored_subject = tokio::task::spawn_blocking(move || -> AppResult<ScoredSubject> {
            let rewritten_source = scryer_rules::rewrite_package_declaration(
                &compute_draft.rego_source,
                &compute_draft_id,
            );
            let retired = scryer_rules::validation::retired_release_input_fields(&rewritten_source)
                .map_err(|error| AppError::validation_refused(crate::error_reasons::rules::VALIDATION_ERROR, format!("rule validation error: {error}")))?;
            if !retired.is_empty() {
                return Err(AppError::validation_refused(crate::error_reasons::rules::SOURCE_USES_RETIRED_FIELDS, format!(
                    "Rule source uses retired fields: {}. Update the source before testing or enabling it.",
                    retired.join(", ")
                )));
            }
            let validation =
                scryer_rules::validation::validate_user_rule(&rewritten_source, &compute_draft_id)
                    .map_err(|error| {
                        AppError::validation_refused(crate::error_reasons::rules::VALIDATION_ERROR, format!("rule validation error: {error}"))
                    })?;
            if !validation.valid {
                return Err(AppError::validation_refused(crate::error_reasons::rules::VALIDATION_FAILED, format!(
                    "Rule validation failed:\n- {}",
                    validation.errors.join("\n- ")
                )));
            }
            let mut rule_sets = rule_sets;
            // A saved rule already sits in the enabled snapshot under its own
            // identity; only a draft needs to be appended for evaluation.
            if !saved_rule_test {
                rule_sets.push(RuleSet {
                    id: compute_draft_id.clone(),
                    name: compute_draft.name.clone(),
                    description: compute_draft.description.clone(),
                    rego_source: rewritten_source,
                    enabled: compute_draft.enabled,
                    priority: compute_draft.priority,
                    evaluation_phase: draft_phase,
                    exclusive_group: draft_exclusive_group,
                    disabled_reason_code: None,
                    disabled_reason: None,
                    applied_facets: compute_draft.applied_facets.clone(),
                    created_at: now,
                    updated_at: now,
                    is_managed: false,
                    managed_key: None,
                    managed_tag_filter: draft_tags,
                });
            }
            let engine = AppUseCase::build_user_rules_engine_for_purpose(
                rule_sets, plugin_policies, super::metrics::Purpose::Preview,
            )?;
            match subject {
                PreviewSubject::Release {
                    name,
                    size_bytes,
                    listing,
                } => {
                    let raw_parsed = crate::release_parser::parse_release_metadata_for_target(
                        &name,
                        &crate::release_parser::build_release_parse_context_for_title(
                            &compute_title,
                            &episodes,
                            Some(compute_title.facet.as_str()),
                        ),
                    );
                    let coverage = crate::acquisition_coverage::resolve_release_coverage(
                        &raw_parsed,
                        &episodes,
                        &collections,
                        compute_episode.as_ref(),
                    );
                    let size_basis = crate::acquisition_coverage::coverage_size_basis(
                        &coverage,
                        &raw_parsed,
                        &episodes,
                        context.default_runtime_minutes(),
                    );
                    let is_filler = compute_episode.as_ref().is_some_and(|item| item.is_filler);
                    let preview_context = crate::canonical_scoring::ScoringContext {
                        rules: (!engine.is_empty()).then_some(&engine),
                        ..context.view(size_basis, is_filler)
                    };
                    let facts = RuleSetTestListingFacts::from_facts(&listing);
                    let listing = *listing;
                    let parsed = crate::canonical_scoring::canonical_parse(
                        &raw_parsed,
                        Some(&listing),
                        &preview_context,
                    );
                    let preview = crate::canonical_scoring::score_release_preview(
                        &crate::canonical_scoring::ReleaseEvidence::announced(
                            raw_parsed,
                            size_bytes,
                        )
                        .with_listing(Some(listing)),
                        &preview_context,
                    );
                    Ok(ScoredSubject {
                        release_name: name,
                        media_file_id: None,
                        parsed,
                        size_bytes,
                        listing: Some(facts),
                        preview,
                    })
                }
                PreviewSubject::StoredFile {
                    file,
                    episode_ids,
                    scoring_episode_ids,
                } => {
                    // The same basis the file's landed bar is derived on.
                    let size_basis = if episode_ids.is_empty() {
                        crate::quality_profile::CoverageSizeBasis::default()
                    } else {
                        crate::acquisition_coverage::episode_span_size_basis(
                            &episodes,
                            &episode_ids,
                            context.default_runtime_minutes(),
                        )
                    };
                    // Scored exactly as its landed bar is: the file's whole
                    // span as the size basis, never a filler view, and a disc
                    // scoped to the episodes being judged.
                    let preview_context = crate::canonical_scoring::ScoringContext {
                        rules: (!engine.is_empty()).then_some(&engine),
                        ..context.view(size_basis, false)
                    };
                    let preview = if scoring_episode_ids.is_empty() {
                        crate::canonical_scoring::score_media_file_preview(&file, &preview_context)
                    } else {
                        crate::canonical_scoring::score_media_file_for_episodes_preview(
                            &file,
                            &scoring_episode_ids,
                            &preview_context,
                        )
                    };
                    let listing = ListingFacts::grabbed_from_json(
                        file.release_listing_json.as_deref(),
                    );
                    Ok(ScoredSubject {
                        release_name: stored_file_release_name(&file),
                        media_file_id: Some(file.id.clone()),
                        parsed: crate::canonical_scoring::canonical_parse(
                            &crate::canonical_scoring::announced_parse_from_media_file(&file),
                            listing.as_ref(),
                            &preview_context,
                        ),
                        size_bytes: Some(crate::canonical_scoring::size_basis_bytes(
                            file.size_bytes,
                            file.announced_size_bytes,
                        )),
                        listing: listing.as_ref().map(RuleSetTestListingFacts::from_facts),
                        preview,
                    })
                }
            }
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("scoring preview worker failed: {error}"))
        })??;

        let ScoredSubject {
            release_name,
            media_file_id,
            parsed,
            size_bytes,
            listing,
            preview,
        } = scored_subject;
        // The pass that set the score: the analyzed one for a stored file
        // that was probed, the announced one otherwise.
        let decision = preview
            .scored
            .analyzed_decision
            .as_ref()
            .unwrap_or(&preview.scored.announced_decision);
        let mut errors = preview
            .rule_errors
            .iter()
            .map(|error| RuleSetTestError {
                code: "rule_evaluation_error".into(),
                message: error.message.clone(),
                rule_set_id: Some(error.rule_set_id.clone()),
            })
            .collect::<Vec<_>>();
        if let Some(error) = preview.engine_error.as_ref() {
            errors.push(RuleSetTestError {
                code: "rules_engine_error".into(),
                message: error.clone(),
                rule_set_id: None,
            });
        }
        let mut rule_sets = preview_rule_sets(
            &decision.scoring_log,
            &preview.rule_errors,
            &draft_id,
            !saved_rule_test,
        );
        if let Some(rule) = saved_rule_result {
            ensure_saved_rule_result(&mut rule_sets, &rule, title.facet.as_str());
        }
        let draft_contribution = preview_draft_contribution(
            &draft,
            &draft_id,
            title.facet.as_str(),
            &decision.scoring_log,
            &preview.rule_errors,
        );
        let library_name = self
            .services
            .catalog
            .libraries
            .get_by_id(&title.library_id)
            .await?
            .map(|library| library.name);

        Ok(RuleSetTestResult {
            release_name,
            media_file_id,
            listing,
            score: preview.scored.total,
            allowed: decision.allowed,
            blocked: !decision.allowed,
            minimum_score_met: !decision
                .block_codes
                .iter()
                .any(|code| code == "score_below_minimum"),
            profile_name: profile.name,
            context: RuleSetTestContext {
                title_name: title.name,
                library_name,
                facet: title.facet.as_str().to_string(),
                language: title.language,
                tags: title.tags,
                episode_label: episode.and_then(|item| item.episode_label.or(item.title)),
            },
            parsed: RuleSetTestParsed {
                quality: parsed.quality,
                source: parsed.source.map(|source| source.to_string()),
                season: parsed
                    .episode
                    .as_ref()
                    .and_then(|episode| episode.season)
                    .map(|season| season.to_string()),
                episode: parsed
                    .episode
                    .as_ref()
                    .and_then(|episode| episode.first_episode())
                    .map(|episode| episode.to_string()),
                edition: parsed.edition,
                size_bytes,
                release_group: parsed.release_group,
                video_codec: parsed.video_codec.map(|codec| codec.to_string()),
                audio: parsed.audio.map(|audio| audio.to_string()),
                year: parsed.year,
                audio_languages: parsed.languages_audio,
            },
            rule_sets,
            draft_contribution,
            errors,
        })
    }

    async fn require_preview_source_rule(&self, id: &str) -> AppResult<RuleSet> {
        self.services
            .customization
            .rule_sets
            .get_rule_set(id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::rules::SET_NOT_FOUND,
                    format!("rule set {id}"),
                )
            })
    }
}

/// Trim the stored-file id once, so a blank id means "no stored file" in both
/// validation and the loader.
fn normalize_preview_subject(request: &mut RuleSetTestRequest) {
    request.media_file_id = request
        .media_file_id
        .take()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
}

fn validate_preview_request(request: &RuleSetTestRequest) -> AppResult<()> {
    const MAX_RELEASE_NAME_BYTES: usize = 4 * 1024;
    const MAX_INDEXER_LANGUAGES: usize = 64;
    const MAX_INDEXER_LANGUAGE_BYTES: usize = 64;
    if request.title_id.trim().is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::TITLE_REQUIRED,
            "a title is required",
        ));
    }
    let release_name = request
        .release_name
        .as_deref()
        .filter(|name| !name.trim().is_empty());
    let media_file_id = request
        .media_file_id
        .as_deref()
        .filter(|id| !id.trim().is_empty());
    match (release_name, media_file_id) {
        (Some(_), Some(_)) => {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::TEST_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_NOT_BOTH,
                "test either a release name or a stored media file, not both",
            ));
        }
        (None, None) => {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::TITLE_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_REQUIRED,
                "title and either a release name or a stored media file are required",
            ));
        }
        (Some(name), None) if name.len() > MAX_RELEASE_NAME_BYTES => {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::RELEASE_NAME_TOO_LARGE_SCORING_PREVIEW,
                "release name is too large for a scoring preview",
            ));
        }
        (None, Some(_)) if request.size_bytes.is_some() || !request.listing.is_empty() => {
            return Err(AppError::validation_refused(
                crate::error_reasons::rules::STORED_MEDIA_FILE_SCORED_OWN_SIZE_LISTING_FACTS,
                "a stored media file is scored from its own size and listing facts; \
                 size and listing inputs apply only to a release name",
            ));
        }
        _ => {}
    }
    if request.size_bytes.is_some_and(|size| size < 0) {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::SIZE_BYTES_CANNOT_NEGATIVE,
            "size bytes cannot be negative",
        ));
    }
    let listing = &request.listing;
    if let Some(raw) = listing.published_at.as_deref()
        && crate::quality_profile::parse_published_at(raw.trim()).is_none()
    {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::PUBLISHED_INVALID,
            format!("published at must be an RFC 2822 or RFC 3339 timestamp, got {raw:?}"),
        ));
    }
    if listing.thumbs_up.is_some_and(|votes| votes < 0)
        || listing.thumbs_down.is_some_and(|votes| votes < 0)
    {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::VOTES_CANNOT_NEGATIVE,
            "votes cannot be negative",
        ));
    }
    if let Some(languages) = listing.indexer_languages.as_ref()
        && (languages.len() > MAX_INDEXER_LANGUAGES
            || languages
                .iter()
                .any(|language| language.len() > MAX_INDEXER_LANGUAGE_BYTES))
    {
        return Err(AppError::validation_refused(
            crate::error_reasons::rules::INDEXER_LANGUAGES_TOO_LARGE_SCORING_PREVIEW,
            "indexer languages are too large for a scoring preview",
        ));
    }
    Ok(())
}

/// The name a stored file's release is shown as: the release it was grabbed
/// as, else its file name.
fn stored_file_release_name(file: &crate::TitleMediaFile) -> String {
    if let Some(title) = file
        .grabbed_release_title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
    {
        return title.to_string();
    }
    crate::stored_paths::stored_path_to_path_buf(&file.file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| file.file_path.clone())
}

/// One scored preview subject, carried out of the blocking worker.
struct ScoredSubject {
    release_name: String,
    media_file_id: Option<String>,
    parsed: crate::ParsedReleaseMetadata,
    size_bytes: Option<i64>,
    listing: Option<RuleSetTestListingFacts>,
    preview: crate::canonical_scoring::ScoredReleasePreview,
}

fn is_rule_diagnostic(
    entry: &crate::quality_profile::ScoringEntry,
    errors: &[scryer_rules::RuleEvalError],
) -> bool {
    if entry.delta != 0 || !matches!(entry.code.as_str(), "user_rule_error" | "system_rule_error") {
        return false;
    }
    match &entry.source {
        crate::quality_profile::ScoringSource::UserRule { id, .. }
        | crate::quality_profile::ScoringSource::SystemRule { id, .. } => {
            errors.iter().any(|error| &error.rule_set_id == id)
        }
        crate::quality_profile::ScoringSource::Builtin => false,
    }
}

fn preview_rule_sets(
    entries: &[crate::quality_profile::ScoringEntry],
    errors: &[scryer_rules::RuleEvalError],
    draft_id: &str,
    is_draft: bool,
) -> Vec<RuleSetTestRuleSetResult> {
    use std::collections::BTreeMap;
    let mut results = BTreeMap::<String, RuleSetTestRuleSetResult>::new();
    for entry in entries {
        if is_rule_diagnostic(entry, errors) {
            continue;
        }
        let (id, name, origin) = match &entry.source {
            crate::quality_profile::ScoringSource::Builtin => {
                ("builtin", "Built-in scoring", "builtin")
            }
            crate::quality_profile::ScoringSource::UserRule { id, name } => {
                (id.as_str(), name.as_str(), "user")
            }
            crate::quality_profile::ScoringSource::SystemRule { id, name } => {
                (id.as_str(), name.as_str(), "system")
            }
        };
        let result = results
            .entry(id.to_string())
            .or_insert_with(|| RuleSetTestRuleSetResult {
                rule_set_id: (id != "builtin").then(|| id.to_string()),
                rule_set_name: name.to_string(),
                origin: origin.into(),
                score: 0,
                matched: false,
                blocked: false,
                is_draft: is_draft && id == draft_id,
                messages: Vec::new(),
                entries: Vec::new(),
            });
        result.matched = true;
        result.blocked |= entry.kind != crate::quality_profile::ScoringEntryKind::ScoreContribution;
        result.entries.push(RuleSetTestEntry {
            code: entry.code.clone(),
            delta: entry.delta,
            blocked: entry.kind != crate::quality_profile::ScoringEntryKind::ScoreContribution,
            kind: entry.kind,
        });
        result.score = crate::quality_profile::sum_score_deltas(
            result.entries.iter().map(|entry| entry.delta),
        );
    }
    for error in errors {
        let result =
            results
                .entry(error.rule_set_id.clone())
                .or_insert_with(|| RuleSetTestRuleSetResult {
                    rule_set_id: Some(error.rule_set_id.clone()),
                    rule_set_name: error.rule_set_name.clone(),
                    origin: match error.origin {
                        scryer_rules::PolicyOrigin::User => "user",
                        scryer_rules::PolicyOrigin::System => "system",
                    }
                    .into(),
                    score: 0,
                    matched: false,
                    blocked: false,
                    is_draft: is_draft && error.rule_set_id == draft_id,
                    messages: Vec::new(),
                    entries: Vec::new(),
                });
        // A probed file runs rules once per evidence pass; one message each.
        if !result.messages.contains(&error.message) {
            result.messages.push(error.message.clone());
        }
    }
    results.into_values().collect()
}

fn ensure_saved_rule_result(
    results: &mut Vec<RuleSetTestRuleSetResult>,
    rule: &SavedRulePreviewMetadata,
    facet: &str,
) {
    if results
        .iter()
        .any(|result| result.rule_set_id.as_deref() == Some(&rule.id))
    {
        return;
    }
    let applies = rule.applied_facets.is_empty()
        || rule
            .applied_facets
            .iter()
            .any(|rule_facet| rule_facet.as_str() == facet);
    let mut messages = Vec::new();
    if !rule.enabled {
        messages.push("rule is disabled".to_string());
    }
    if !applies {
        messages.push(format!("rule does not apply to {facet}"));
    }
    if messages.is_empty() {
        messages.push("rule did not match this release".to_string());
    }
    results.push(RuleSetTestRuleSetResult {
        rule_set_id: Some(rule.id.clone()),
        rule_set_name: rule.name.clone(),
        origin: if rule.is_managed { "system" } else { "user" }.into(),
        score: 0,
        matched: false,
        blocked: false,
        is_draft: false,
        messages,
        entries: Vec::new(),
    });
}

fn preview_draft_contribution(
    draft: &RuleSetTestDraft,
    draft_id: &str,
    facet: &str,
    entries: &[crate::quality_profile::ScoringEntry],
    errors: &[scryer_rules::RuleEvalError],
) -> RuleSetTestDraftContribution {
    let applies = draft.applied_facets.is_empty()
        || draft
            .applied_facets
            .iter()
            .any(|item| item.as_str() == facet);
    let matching = entries
        .iter()
        .filter(|entry| !is_rule_diagnostic(entry, errors))
        .filter_map(|entry| match &entry.source {
            crate::quality_profile::ScoringSource::UserRule { id, .. } if id == draft_id => {
                Some(entry.delta)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let error = errors
        .iter()
        .find(|error| error.rule_set_id == draft_id)
        .map(|error| error.message.clone());
    let message = if !draft.enabled {
        Some("The draft is disabled, so it was validated but not evaluated.".into())
    } else if !applies {
        Some(format!("The draft does not apply to the {facet} facet."))
    } else {
        error
    };
    RuleSetTestDraftContribution {
        score: crate::quality_profile::sum_score_deltas(matching.iter().copied()),
        matched: !matching.is_empty(),
        blocked: false,
        applies,
        enabled: draft.enabled,
        message,
    }
}
