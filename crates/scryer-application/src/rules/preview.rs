//! Read-only title-aware scoring previews for the rule editor.

use chrono::Utc;
use scryer_domain::{AppPermission, Id, MediaFacet, RuleSet, User};

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
    pub draft: RuleSetTestDraft,
    pub edit_rule_set_id: Option<String>,
    pub copy_source_rule_set_id: Option<String>,
    pub copy_disables_source: bool,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub release_name: String,
    pub size_bytes: Option<i64>,
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
        request: RuleSetTestRequest,
    ) -> AppResult<RuleSetTestResult> {
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await?;
        validate_preview_request(&request)?;

        let title = self
            .get_title(actor, &request.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", request.title_id)))?;

        let episode = match request.episode_id.as_deref() {
            Some(episode_id) => {
                let episode = self
                    .services
                    .catalog
                    .shows
                    .get_episode_by_id(episode_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("episode {episode_id}")))?;
                if episode.title_id != title.id {
                    return Err(AppError::Validation(format!(
                        "episode {episode_id} does not belong to title {}",
                        title.id
                    )));
                }
                Some(episode)
            }
            None => None,
        };
        if title.facet == MediaFacet::Movie && episode.is_some() {
            return Err(AppError::Validation(
                "an episode cannot be selected when testing a movie rule".into(),
            ));
        }
        if matches!(title.facet, MediaFacet::Series | MediaFacet::Anime) && episode.is_none() {
            return Err(AppError::Validation(
                "an episode is required when testing a series or anime rule".into(),
            ));
        }

        if request.edit_rule_set_id.is_some() && request.copy_source_rule_set_id.is_some() {
            return Err(AppError::Validation(
                "a preview cannot edit and copy a rule at the same time".into(),
            ));
        }
        if request.copy_disables_source && request.copy_source_rule_set_id.is_none() {
            return Err(AppError::Validation(
                "copy_disables_source requires a copy source rule set".into(),
            ));
        }
        let draft_id = request
            .edit_rule_set_id
            .clone()
            .unwrap_or_else(|| Id::new_rego_safe().0);

        // Take one consistent policy snapshot while mutations are excluded.
        // Compilation happens after this scope, so a slow/large preview never
        // holds the mutation lock or replaces the production engine.
        let (rule_sets, plugin_policies) = {
            let _mutation = self.services.customization.rule_mutation_lock.lock().await;
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
                    return Err(AppError::Validation(
                        "managed or tracked rules must be copied before editing".into(),
                    ));
                }
            }
            let mut enabled = self
                .services
                .customization
                .rule_sets
                .list_enabled_rule_sets()
                .await?;
            if let Some(source) = edit_source {
                enabled.retain(|rule_set| rule_set.id != source.id);
            }
            if let Some(source) = copy_source.as_ref() {
                let tracked = self
                    .services
                    .customization
                    .rule_sets
                    .find_rule_pack_installation_by_rule_set_id(&source.id)
                    .await?
                    .is_some();
                if tracked && !request.copy_disables_source {
                    return Err(AppError::Validation(
                        "copying a tracked rule requires disabling its source".into(),
                    ));
                }
                if tracked && request.copy_disables_source {
                    enabled.retain(|rule_set| rule_set.id != source.id);
                }
            }
            let plugin_policies = self
                .services
                .integrations
                .plugin_provider
                .available()
                .map(|provider| provider.scoring_policies())
                .unwrap_or_default();
            (enabled, plugin_policies)
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
        let compute_draft = request.draft.clone();
        let compute_draft_id = draft_id.clone();
        let compute_release_name = request.release_name.clone();
        let compute_size_bytes = request.size_bytes;
        let (parsed, preview) = tokio::task::spawn_blocking(move || -> AppResult<_> {
            let rewritten_source = scryer_rules::rewrite_package_declaration(
                &compute_draft.rego_source,
                &compute_draft_id,
            );
            let validation =
                scryer_rules::validation::validate_user_rule(&rewritten_source, &compute_draft_id)
                    .map_err(|error| {
                        AppError::Validation(format!("rule validation error: {error}"))
                    })?;
            if !validation.valid {
                return Err(AppError::Validation(format!(
                    "Rule validation failed:\n- {}",
                    validation.errors.join("\n- ")
                )));
            }
            let now = Utc::now();
            let mut rule_sets = rule_sets;
            rule_sets.push(RuleSet {
                id: compute_draft_id.clone(),
                name: compute_draft.name.clone(),
                description: compute_draft.description.clone(),
                rego_source: rewritten_source,
                enabled: compute_draft.enabled,
                priority: compute_draft.priority,
                applied_facets: compute_draft.applied_facets.clone(),
                created_at: now,
                updated_at: now,
                is_managed: false,
                managed_key: None,
                managed_tag_filter: None,
            });
            let engine = AppUseCase::build_user_rules_engine(rule_sets, plugin_policies)?;
            let raw_parsed = crate::release_parser::parse_release_metadata_for_target(
                &compute_release_name,
                &crate::release_parser::build_release_parse_context_for_title(
                    &compute_title,
                    &episodes,
                    Some(compute_title.facet.as_str()),
                ),
            );
            let parsed = crate::quality::canonical_context::announced_metadata_for_title(
                &compute_title,
                &raw_parsed,
                context.required_audio_languages(),
                None,
            );
            let coverage = crate::acquisition_coverage::resolve_release_coverage(
                &raw_parsed,
                &episodes,
                &collections,
                compute_episode.as_ref(),
            );
            let size_basis = crate::acquisition_coverage::coverage_size_basis(
                &coverage,
                &parsed,
                &episodes,
                context.default_runtime_minutes(),
            );
            let preview_context = context.view(
                size_basis,
                compute_episode.as_ref().is_some_and(|item| item.is_filler),
            );
            let preview_context = crate::canonical_scoring::ScoringContext {
                rules: (!engine.is_empty()).then_some(&engine),
                ..preview_context
            };
            let preview = crate::canonical_scoring::score_release_preview(
                &crate::canonical_scoring::ReleaseEvidence::announced(
                    parsed.clone(),
                    compute_size_bytes,
                ),
                &preview_context,
            );
            Ok((parsed, preview))
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("scoring preview worker failed: {error}"))
        })??;

        let mut errors = preview
            .rule_errors
            .iter()
            .map(|error| RuleSetTestError {
                code: "rule_evaluation_error".into(),
                message: error.message.clone(),
                rule_set_id: Some(error.rule_set_id.clone()),
            })
            .collect::<Vec<_>>();
        if let Some(error) = preview.engine_error {
            errors.push(RuleSetTestError {
                code: "rules_engine_error".into(),
                message: error,
                rule_set_id: None,
            });
        }
        let rule_sets = preview_rule_sets(
            &preview.scored.announced_decision.scoring_log,
            &preview.rule_errors,
            &draft_id,
        );
        let draft_contribution = preview_draft_contribution(
            &request.draft,
            &draft_id,
            title.facet.as_str(),
            &preview.scored.announced_decision.scoring_log,
            &preview.rule_errors,
        );
        let decision = &preview.scored.announced_decision;
        let library_name = self
            .services
            .catalog
            .libraries
            .get_by_id(&title.library_id)
            .await?
            .map(|library| library.name);

        Ok(RuleSetTestResult {
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
                size_bytes: request.size_bytes,
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
            .ok_or_else(|| AppError::NotFound(format!("rule set {id}")))
    }
}

fn validate_preview_request(request: &RuleSetTestRequest) -> AppResult<()> {
    const MAX_RELEASE_NAME_BYTES: usize = 4 * 1024;
    if request.title_id.trim().is_empty() || request.release_name.trim().is_empty() {
        return Err(AppError::Validation(
            "title and release name are required".into(),
        ));
    }
    if request.release_name.len() > MAX_RELEASE_NAME_BYTES {
        return Err(AppError::Validation(
            "release name is too large for a scoring preview".into(),
        ));
    }
    if request.size_bytes.is_some_and(|size| size < 0) {
        return Err(AppError::Validation("size bytes cannot be negative".into()));
    }
    Ok(())
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
                is_draft: id == draft_id,
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
                    is_draft: error.rule_set_id == draft_id,
                    messages: Vec::new(),
                    entries: Vec::new(),
                });
        result.messages.push(error.message.clone());
    }
    results.into_values().collect()
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
