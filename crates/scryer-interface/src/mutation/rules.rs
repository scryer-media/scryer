use crate::context::{actor_from_ctx, app_from_ctx, require_config_app_permission, to_gql_error};
use crate::types::*;
use async_graphql::{Context, Error, ID, Object, Result as GqlResult};
use scryer_domain::{AppPermission, RulePackInstallation, RuleSet, User};
use std::collections::HashMap;

async fn require_tracked_rule_pack_permission(ctx: &Context<'_>) -> GqlResult<User> {
    require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
    require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await
}

fn from_tracked_rule_pack(
    installation: RulePackInstallation,
    available_version: Option<String>,
    auto_update_available: bool,
    rule_sets: &HashMap<String, RuleSet>,
) -> TrackedRulePackPayload {
    TrackedRulePackPayload {
        pack_id: installation.pack_id,
        name: installation.name,
        version: installation.version,
        digest: installation.digest,
        revision: installation.revision.into(),
        auto_update: installation.auto_update,
        available_version,
        auto_update_available,
        last_error: installation.last_error,
        last_updated: installation.last_updated,
        members: installation
            .members
            .into_iter()
            .map(|member| {
                let rule_set = rule_sets.get(&member.rule_set_id);
                TrackedRulePackMemberPayload {
                    template_id: member.template_id,
                    rule_set_id: ID::from(member.rule_set_id),
                    removed: member.removed,
                    enabled: rule_set.map(|rule_set| rule_set.enabled),
                    priority: rule_set.map(|rule_set| rule_set.priority),
                    name: rule_set.map(|rule_set| rule_set.name.clone()),
                    description: rule_set.map(|rule_set| rule_set.description.clone()),
                    applied_facets: rule_set
                        .map(|rule_set| {
                            rule_set
                                .applied_facets
                                .iter()
                                .map(|facet| facet.as_str().to_string())
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect(),
    }
}

async fn tracked_rule_pack_payload(
    app: &scryer_application::AppUseCase,
    actor: &User,
    pack_id: &str,
) -> GqlResult<TrackedRulePackPayload> {
    let installation = app
        .list_tracked_rule_packs(actor)
        .await
        .map_err(to_gql_error)?
        .into_iter()
        .find(|installation| installation.pack_id == pack_id)
        .ok_or_else(|| Error::new("tracked rule pack was not found after mutation"))?;
    let rule_sets = app
        .list_rule_sets(actor)
        .await
        .map_err(to_gql_error)?
        .into_iter()
        .map(|rule_set| (rule_set.id.clone(), rule_set))
        .collect::<HashMap<_, _>>();
    let available_version = app
        .rule_pack_update_candidate(actor, &installation.pack_id, &installation.version, false)
        .await
        .unwrap_or(None)
        .map(|candidate| candidate.version);
    let auto_update_available = if installation.auto_update {
        app.rule_pack_update_candidate(actor, &installation.pack_id, &installation.version, true)
            .await
            .unwrap_or(None)
            .is_some()
    } else {
        false
    };
    Ok(from_tracked_rule_pack(
        installation,
        available_version,
        auto_update_available,
        &rule_sets,
    ))
}

fn parse_facets(input: Option<Vec<String>>) -> Vec<scryer_domain::MediaFacet> {
    input
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| scryer_domain::MediaFacet::parse(&s))
        .collect()
}

#[derive(Default)]
pub(crate) struct RulesMutations;

#[expect(
    clippy::too_many_arguments,
    reason = "GraphQL pack settings and copy methods expose individual authoring fields"
)]
#[Object]
impl RulesMutations {
    /// Create a catalog rule set with optional facet scope, priority, and enabled state.
    async fn create_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Rule name, source, optional description and facet scope, priority, and enabled state."
        )]
        input: CreateRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set = app
            .create_rule_set(
                &actor,
                input.name,
                input.description.unwrap_or_default(),
                input.rego_source,
                parse_facets(input.applied_facets),
                input.priority.unwrap_or(0),
                input.enabled,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    /// Patch a rule set while preserving omitted fields.
    async fn update_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Rule-set identity and optional replacement source, metadata, facet scope, priority, or managed tag filter."
        )]
        input: UpdateRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set = app
            .update_rule_set(
                &actor,
                String::from(input.id),
                input.name,
                input.description,
                input.rego_source,
                input.applied_facets.map(|f| parse_facets(Some(f))),
                input.priority,
                input.managed_tag_filter,
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    /// Delete a rule set.
    async fn delete_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Rule-set identity to delete.")] id: ID,
    ) -> GqlResult<DeleteRuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let id = id.to_string();
        app.delete_rule_set(&actor, &id)
            .await
            .map_err(to_gql_error)?;

        Ok(DeleteRuleSetPayload { id: ID::from(id) })
    }

    /// Set whether a rule set is enabled.
    async fn toggle_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Rule-set identity and desired enabled state.")] input: ToggleRuleSetInput,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set = app
            .toggle_rule_set(&actor, input.id.as_ref(), input.enabled)
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_rule_set(rule_set))
    }

    /// Replace required audio languages for one title and facet.
    async fn set_title_required_audio(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Title identity, facet, and required audio-language codes.")]
        input: SetTitleRequiredAudioInput,
    ) -> GqlResult<SetTitleRequiredAudioPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = String::from(input.title_id);
        let facet_value = input.facet;
        let facet = input.facet.into_domain();
        let languages = input.languages;
        app.set_title_required_audio(&actor, &title_id, facet.as_str(), languages.clone())
            .await
            .map_err(to_gql_error)?;
        Ok(SetTitleRequiredAudioPayload {
            title_id: ID::from(title_id),
            facet: facet_value,
            languages,
            updated: true,
        })
    }

    /// Validate rule source without saving it, using a supplied or temporary rule identity.
    async fn validate_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Rule source and optional rule-set identity used for validation.")]
        input: ValidateRuleSetInput,
    ) -> GqlResult<RuleValidationResultPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;

        let rule_set_id = input
            .rule_set_id
            .map(String::from)
            .unwrap_or_else(|| "r_validation_test".to_string());
        let result = app
            .validate_rule_set(&actor, &input.rego_source, &rule_set_id)
            .await
            .map_err(to_gql_error)?;

        Ok(RuleValidationResultPayload {
            valid: result.valid,
            errors: result.errors,
        })
    }

    /// Evaluate the current unsaved editor draft against title-aware release facts without saving it.
    async fn test_rule_set(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Current editor draft, copy context, selected title and episode, release name, and optional size. This is a scoring preview only."
        )]
        input: TestRuleSetInput,
    ) -> GqlResult<TestRuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor =
            require_config_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let request = scryer_application::RuleSetTestRequest {
            draft: scryer_application::RuleSetTestDraft {
                name: input.draft.name,
                description: input.draft.description,
                rego_source: input.draft.rego_source,
                enabled: input.draft.enabled,
                priority: input.draft.priority,
                applied_facets: parse_facets(Some(input.draft.applied_facets)),
            },
            edit_rule_set_id: input.edit_rule_set_id.map(String::from),
            copy_source_rule_set_id: input.copy_source_rule_set_id.map(String::from),
            copy_disables_source: input.copy_disables_source,
            title_id: input.title_id.into(),
            episode_id: input.episode_id.map(String::from),
            release_name: input.release_name,
            size_bytes: input.size_bytes.map(Into::into),
        };
        let result = app
            .test_rule_set(&actor, request)
            .await
            .map_err(to_gql_error)?;

        Ok(TestRuleSetPayload {
            score: result.score,
            allowed: result.allowed,
            blocked: result.blocked,
            minimum_score_met: result.minimum_score_met,
            profile_name: result.profile_name,
            context: RuleSetTestContextPayload {
                title_name: result.context.title_name,
                library_name: result.context.library_name,
                facet: result.context.facet,
                language: result.context.language,
                tags: result.context.tags,
                episode_label: result.context.episode_label,
            },
            parsed: RuleSetTestParsedPayload {
                release_group: result.parsed.release_group,
                quality: result.parsed.quality,
                source: result.parsed.source,
                season: result.parsed.season,
                episode: result.parsed.episode,
                edition: result.parsed.edition,
                video_codec: result.parsed.video_codec,
                audio: result.parsed.audio,
                year: result.parsed.year,
                audio_languages: result.parsed.audio_languages,
                size_bytes: result.parsed.size_bytes.map(Into::into),
            },
            rule_sets: result
                .rule_sets
                .into_iter()
                .map(|rule_set| RuleSetTestRuleSetPayload {
                    rule_set_id: rule_set.rule_set_id,
                    rule_set_name: rule_set.rule_set_name,
                    origin: rule_set.origin,
                    score: rule_set.score,
                    matched: rule_set.matched,
                    blocked: rule_set.blocked,
                    is_draft: rule_set.is_draft,
                    entries: rule_set
                        .entries
                        .into_iter()
                        .map(|entry| RuleSetTestEntryPayload {
                            code: entry.code,
                            delta: entry.delta,
                            blocked: entry.blocked,
                        })
                        .collect(),
                    messages: rule_set.messages,
                })
                .collect(),
            draft_contribution: RuleSetTestDraftContributionPayload {
                score: result.draft_contribution.score,
                matched: result.draft_contribution.matched,
                blocked: result.draft_contribution.blocked,
                applies: result.draft_contribution.applies,
                enabled: result.draft_contribution.enabled,
                message: result.draft_contribution.message,
            },
            errors: result
                .errors
                .into_iter()
                .map(|error| RuleSetTestErrorPayload {
                    code: error.code,
                    message: error.message,
                    rule_set_id: error.rule_set_id,
                })
                .collect(),
        })
    }

    /// Install the whole verified pack with selected templates enabled.
    async fn install_tracked_rule_pack(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Immutable registry rule-pack ID.")] pack_id: String,
        #[graphql(desc = "Template IDs to enable. An empty list installs every rule disabled.")]
        template_ids: Vec<String>,
    ) -> GqlResult<TrackedRulePackPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_tracked_rule_pack_permission(ctx).await?;
        app.install_tracked_rule_pack(&actor, &pack_id, &template_ids)
            .await
            .map_err(to_gql_error)?;
        tracked_rule_pack_payload(&app, &actor, &pack_id).await
    }

    /// Preview the exact template changes from the latest compatible source version.
    async fn preview_tracked_rule_pack_update(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Installed rule-pack ID to compare with the registry.")] pack_id: String,
    ) -> GqlResult<TrackedRulePackPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_tracked_rule_pack_permission(ctx).await?;
        let preview = app
            .preview_tracked_rule_pack_update(&actor, &pack_id)
            .await
            .map_err(to_gql_error)?;
        Ok(TrackedRulePackPreviewPayload {
            version: preview.version,
            digest: preview.digest,
            revision: preview.revision.into(),
            added_template_ids: preview.added,
            changed_template_ids: preview.changed,
            removed_template_ids: preview.removed,
        })
    }

    /// Apply a previously reviewed immutable source version to a tracked rule pack.
    async fn update_tracked_rule_pack(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Installed rule-pack ID to update.")] pack_id: String,
        #[graphql(desc = "Reviewed immutable source version.")] version: String,
        #[graphql(desc = "Verified digest for the reviewed source version.")] digest: String,
        #[graphql(desc = "Revision returned by the update preview.")] revision: Long,
    ) -> GqlResult<TrackedRulePackPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_tracked_rule_pack_permission(ctx).await?;
        app.update_tracked_rule_pack(&actor, &pack_id, &version, &digest, revision.into())
            .await
            .map_err(to_gql_error)?;
        tracked_rule_pack_payload(&app, &actor, &pack_id).await
    }

    /// Atomically retain editable member settings and the automatic-update preference.
    async fn set_tracked_rule_pack_settings(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Installed rule-pack ID.")] pack_id: String,
        #[graphql(desc = "Template IDs that should be enabled.")] enabled_template_ids: Vec<String>,
        #[graphql(desc = "Local priority for each template whose priority changes.")]
        priorities: Vec<TrackedRulePackPriorityInput>,
        #[graphql(desc = "Whether the scheduled registry refresh may update this pack.")]
        auto_update: bool,
        #[graphql(desc = "Current pack revision required for this mutation.")]
        expected_revision: Long,
    ) -> GqlResult<TrackedRulePackPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_tracked_rule_pack_permission(ctx).await?;
        let priorities = priorities
            .into_iter()
            .map(|priority| (priority.template_id, priority.priority))
            .collect::<Vec<_>>();
        app.set_tracked_rule_pack_settings(
            &actor,
            &pack_id,
            &enabled_template_ids,
            &priorities,
            auto_update,
            expected_revision.into(),
        )
        .await
        .map_err(to_gql_error)?;
        tracked_rule_pack_payload(&app, &actor, &pack_id).await
    }

    /// Copy one tracked rule to an editable custom rule and disable the source binding.
    async fn copy_tracked_rule_pack_rule(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Tracked local rule-set ID to copy.")] rule_set_id: ID,
        #[graphql(desc = "Name for the editable copy.")] name: String,
        #[graphql(desc = "Description for the editable copy.")] description: String,
        #[graphql(desc = "Complete Rego source for the editable copy.")] rego_source: String,
        #[graphql(desc = "Facet scope for the editable copy.")] applied_facets: Vec<String>,
        #[graphql(desc = "Evaluation priority for the editable copy.")] priority: i32,
    ) -> GqlResult<RuleSetPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_tracked_rule_pack_permission(ctx).await?;
        let rule_set = app
            .copy_tracked_rule_pack_rule(
                &actor,
                rule_set_id.as_ref(),
                name,
                description,
                rego_source,
                parse_facets(Some(applied_facets)),
                priority,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(crate::mappers::from_rule_set(rule_set))
    }

    /// Uninstall a tracked rule pack and its owned rules.
    async fn uninstall_tracked_rule_pack(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Installed rule-pack ID to remove.")] pack_id: String,
        #[graphql(desc = "Current pack revision required for this mutation.")] revision: Long,
    ) -> GqlResult<DeleteTrackedRulePackPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_tracked_rule_pack_permission(ctx).await?;
        app.uninstall_tracked_rule_pack(&actor, &pack_id, revision.into())
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteTrackedRulePackPayload { pack_id })
    }
}
