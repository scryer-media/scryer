use super::*;
use scryer_application::rule_import::{
    RuleImportConnection, RuleImportItemState, RuleImportPhase, RuleImportSource, refusal,
};
use scryer_application::rule_import_arr;
use scryer_application::{JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource};
use scryer_infrastructure_workflow::workflow::stores::WorkflowOperationStore;
use tokio::time::{Duration, timeout};
use wiremock::MockServer;
use wiremock::matchers::header;

const FAILURE_BOUND: Duration = Duration::from_secs(120);

fn store(ctx: &TestContext) -> WorkflowOperationStore {
    WorkflowOperationStore::new(ctx.db.datastore())
        .with_encryption_key(ctx.db.encryption_key_state())
}

/// A Maintainerr that answers everything a fetch reads. Its rule user list is
/// missing, as on older versions, and one Radarr server's profiles fail.
async fn maintainerr() -> MockServer {
    let server = MockServer::start().await;
    let routes = [
        ("/api/settings/version", ResponseTemplate::new(200).set_body_string("2.19.0")),
        (
            "/api/rules",
            ResponseTemplate::new(200).set_body_json(json!([
                {"id": 1, "name": "Synthetic watched movies", "rules": [], "collection": {"radarrSettingsId": 2}},
                {"id": 2, "name": "Synthetic stale series", "rules": [], "collection": {"sonarrSettingsId": 3}}
            ])),
        ),
        ("/api/rules/constants", ResponseTemplate::new(200).set_body_json(json!({"applications": []}))),
        ("/api/rules/users", ResponseTemplate::new(404).set_body_string("Cannot GET")),
        ("/api/servarr/radarr/2/profiles", ResponseTemplate::new(404)),
        (
            "/api/servarr/sonarr/3/profiles",
            ResponseTemplate::new(200).set_body_json(json!([{"id": 1, "name": "Any"}])),
        ),
    ];
    for (route, response) in routes {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(response)
            .mount(&server)
            .await;
    }
    server
}

fn maintainerr_connection(server: &MockServer) -> RuleImportConnection {
    RuleImportConnection {
        source: RuleImportSource::Maintainerr,
        base_url: server.uri(),
        api_key: None,
    }
}

#[tokio::test]
async fn rule_import_fetches_purges_the_connection_and_refuses_an_untranslated_source() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        let actor = ctx
            .users
            .create(User::new_admin("rule-import-operator"))
            .await
            .expect("operator");
        let server = maintainerr().await;
        let connection = maintainerr_connection(&server);
        let run = ctx
            .app
            .start_rule_import(&actor, "maintainerr-first", connection.clone())
            .await
            .expect("accepted");
        assert_eq!(run.job_key, JobKey::ExternalRuleImport);

        let report = ctx
            .app
            .wait_rule_import(&actor, &run.id)
            .await
            .expect("settles");
        assert_eq!(report.phase, RuleImportPhase::Finished);
        assert_eq!(
            report.reason.as_deref(),
            Some(refusal::TRANSLATOR_UNAVAILABLE)
        );
        assert!(report.items.is_empty());
        let fact_codes: Vec<_> = report.facts.iter().map(|fact| fact.code.as_str()).collect();
        assert!(
            fact_codes.contains(&refusal::USERNAMES_UNAVAILABLE),
            "{fact_codes:?}"
        );
        let profiles = report
            .facts
            .iter()
            .find(|fact| fact.code == refusal::PROFILES_UNAVAILABLE)
            .expect("the failed profile read is reported");
        assert_eq!(profiles.details.as_ref().expect("details")["settingsId"], 2);

        let (stored, _) = ctx
            .app
            .rule_import_status(&actor, &run.id)
            .await
            .expect("status");
        assert_eq!(stored.status, JobRunStatus::Failed);
        assert!(stored.completed_at.is_some());
        assert!(
            store(&ctx)
                .private_job_input(&run.id, &actor.id)
                .await
                .expect("input read")
                .is_none(),
            "the connection is purged once the run resolves"
        );

        // The same request key answers with the same run and starts nothing new.
        let requests = server.received_requests().await.expect("requests").len();
        let repeated = ctx
            .app
            .start_rule_import(&actor, "maintainerr-first", connection.clone())
            .await
            .expect("idempotent");
        assert_eq!(repeated.id, run.id);
        assert_eq!(
            server.received_requests().await.expect("requests").len(),
            requests
        );

        let changed = ctx
            .app
            .start_rule_import(
                &actor,
                "maintainerr-first",
                RuleImportConnection {
                    base_url: format!("{}/other", server.uri()),
                    ..connection
                },
            )
            .await
            .expect_err("a reused key cannot change its source");
        assert_eq!(changed.validation_reason(), Some(refusal::REQUEST_CHANGED));
    })
    .await
    .expect("rule import scenario failure bound");
}

#[tokio::test]
async fn rule_import_reports_a_coded_source_failure_and_keeps_no_api_key() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        let actor = ctx
            .users
            .create(User::new_admin("rule-import-sonarr"))
            .await
            .expect("operator");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        for source in [RuleImportSource::Sonarr, RuleImportSource::Radarr] {
            let run = ctx
                .app
                .start_rule_import(
                    &actor,
                    &format!("{source:?}-rejected"),
                    RuleImportConnection {
                        source,
                        base_url: server.uri(),
                        api_key: Some("synthetic-rejected-key".into()),
                    },
                )
                .await
                .expect("accepted");
            let report = ctx
                .app
                .wait_rule_import(&actor, &run.id)
                .await
                .expect("settles");
            assert_eq!(
                report.reason.as_deref(),
                Some(scryer_application::error_reasons::external_import::SOURCE_UNAUTHORIZED)
            );
            let (stored, _) = ctx
                .app
                .rule_import_status(&actor, &run.id)
                .await
                .expect("status");
            let stored_text = format!(
                "{:?}{:?}{:?}",
                stored.progress_json, stored.summary_json, stored.error_text
            );
            assert!(!stored_text.contains("synthetic-rejected-key"));
            assert!(
                store(&ctx)
                    .private_job_input(&run.id, &actor.id)
                    .await
                    .expect("input read")
                    .is_none()
            );
        }
    })
    .await
    .expect("rule import failure scenario bound");
}

#[tokio::test]
async fn rule_import_needs_catalog_settings_permission_and_hides_other_users_runs() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        let owner = ctx
            .users
            .create(User::new_admin("rule-import-owner"))
            .await
            .expect("owner");
        let viewer = ctx
            .users
            .create(User {
                id: Id::new().0,
                username: "rule-import-viewer".into(),
                password_hash: None,
                password_change_required: false,
                account_kind: Default::default(),
                authorization: Default::default(),
            })
            .await
            .expect("viewer");
        let server = maintainerr().await;
        let denied = ctx
            .app
            .start_rule_import(&viewer, "denied", maintainerr_connection(&server))
            .await
            .expect_err("no catalog settings permission");
        assert!(
            matches!(
                denied,
                AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
            ),
            "{denied:?}"
        );

        let run = ctx
            .app
            .start_rule_import(&owner, "owned", maintainerr_connection(&server))
            .await
            .expect("accepted");
        ctx.app
            .wait_rule_import(&owner, &run.id)
            .await
            .expect("settles");
        let other_admin = ctx
            .users
            .create(User::new_admin("rule-import-other-admin"))
            .await
            .expect("other admin");
        assert_eq!(
            ctx.app
                .rule_import_status(&other_admin, &run.id)
                .await
                .expect_err("another user's run")
                .validation_reason(),
            Some(refusal::NOT_OWNED)
        );
    })
    .await
    .expect("rule import permission scenario bound");
}

#[tokio::test]
async fn a_restart_keeps_the_rule_import_report_and_marks_unsaved_items_interrupted() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        let actor = ctx
            .users
            .create(User::new_admin("rule-import-restart"))
            .await
            .expect("operator");
        let store = store(&ctx);
        let now = chrono::Utc::now();
        let report = json!({
            "revision": 4,
            "source": "RADARR",
            "baseUrl": "http://radarr.example.test",
            "phase": "SAVING",
            "items": [
                {"key": "customformat:1", "sourceName": "Synthetic saved", "savedName": "Synthetic saved", "ruleSetId": "rule-set-1", "state": "SAVED", "facts": []},
                {"key": "customformat:2", "sourceName": "Synthetic pending", "state": "PENDING", "facts": []}
            ]
        });
        let run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::ExternalRuleImport,
            operation_type: "external_rule_import".into(),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: Some(report.to_string()),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        store
            .accept_private_job(
                &run,
                "rule-import:interrupted",
                &json!({"source": "RADARR", "baseUrl": "http://radarr.example.test", "apiKey": "synthetic-key"}).to_string(),
            )
            .await
            .expect("seed running import");

        store.reconcile_interrupted_job_runs(&[]).await.expect("boot reconcile");

        let (stored, progress) = ctx.app.rule_import_status(&actor, &run.id).await.expect("status");
        assert_eq!(stored.status, JobRunStatus::Failed);
        assert_eq!(progress.items[0].state, RuleImportItemState::Saved);
        assert_eq!(progress.items[0].rule_set_id.as_deref(), Some("rule-set-1"));
        assert_eq!(progress.items[1].state, RuleImportItemState::Failed);
        assert_eq!(progress.items[1].reason.as_deref(), Some(refusal::INTERRUPTED));
        assert_eq!(progress.reason.as_deref(), Some(refusal::INTERRUPTED));
        assert_eq!(progress.counts.saved, 1);
        assert_eq!(progress.counts.failed, 1);
        assert!(
            store
                .private_job_input(&run.id, &actor.id)
                .await
                .expect("input read")
                .is_none(),
            "a restart purges the stored connection"
        );
    })
    .await
    .expect("rule import restart scenario bound");
}

#[tokio::test]
async fn graphql_starts_a_rule_import_and_reads_its_report() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        let server = maintainerr().await;
        let start = r#"mutation($requestKey: String!, $input: ExternalRuleImportInput!) {
            startExternalRuleImport(requestKey: $requestKey, input: $input) { id jobKey status }
        }"#;
        let body = gql(
            &ctx,
            start,
            json!({
                "requestKey": "graphql-rule-import",
                "input": {"source": "MAINTAINERR", "baseUrl": server.uri(), "apiKey": "ignored-for-maintainerr"}
            }),
        )
        .await;
        assert_no_errors(&body);
        let run = &body["data"]["startExternalRuleImport"];
        assert_eq!(run["jobKey"], "EXTERNAL_RULE_IMPORT");
        let run_id = run["id"].as_str().expect("run id").to_string();

        let actor = ctx.app.find_or_create_default_user().await.expect("default user");
        ctx.app.wait_rule_import(&actor, &run_id).await.expect("settles");

        let status = gql(
            &ctx,
            r#"query($runId: ID!) {
                externalRuleImportStatus(runId: $runId) {
                    runId status active revision source baseUrl phase reason message
                    counts { total pending saved savedNeedsReview savedFailedTranslation skippedExists failed }
                    items { key sourceName savedName ruleSetId state reason message facts { code message details } }
                    facts { code message details }
                }
            }"#,
            json!({ "runId": run_id }),
        )
        .await;
        assert_no_errors(&status);
        let report = &status["data"]["externalRuleImportStatus"];
        assert_eq!(report["runId"], run_id);
        assert_eq!(report["status"], "FAILED");
        assert_eq!(report["active"], false);
        assert_eq!(report["source"], "MAINTAINERR");
        assert_eq!(report["phase"], "FINISHED");
        assert_eq!(report["reason"], refusal::TRANSLATOR_UNAVAILABLE);
        assert_eq!(report["counts"]["total"], 0);
        assert!(
            report["facts"]
                .as_array()
                .expect("facts")
                .iter()
                .any(|fact| fact["code"] == refusal::PROFILES_UNAVAILABLE
                    && fact["details"]["settingsId"] == 2)
        );
        assert!(!status.to_string().contains("ignored-for-maintainerr"));

        let missing_key = gql(
            &ctx,
            start,
            json!({
                "requestKey": "graphql-sonarr-no-key",
                "input": {"source": "SONARR", "baseUrl": server.uri()}
            }),
        )
        .await;
        assert_eq!(
            missing_key["errors"][0]["extensions"]["reason"],
            refusal::API_KEY_REQUIRED,
            "{missing_key}"
        );

        let unknown = gql(
            &ctx,
            r#"query { externalRuleImportStatus(runId: "missing-run") { runId } }"#,
            json!({}),
        )
        .await;
        assert_eq!(
            unknown["errors"][0]["extensions"]["reason"],
            refusal::NOT_OWNED,
            "{unknown}"
        );
    })
    .await
    .expect("graphql rule import scenario bound");
}

fn arr_title_format(id: i64, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "includeCustomFormatWhenRenaming": false,
        "specifications": [{
            "name": "Sample",
            "implementation": "ReleaseTitleSpecification",
            "negate": false,
            "required": true,
            "fields": [{"order": 0, "name": "value", "value": "\\bsample\\b"}]
        }]
    })
}

fn arr_year_format(id: i64, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "specifications": [{
            "name": "Year",
            "implementation": "YearSpecification",
            "negate": false,
            "required": true,
            "fields": [{"name": "min", "value": 2000}, {"name": "max", "value": 2010}]
        }]
    })
}

/// A Radarr that lists `formats` and `profiles` to the synthetic key only.
async fn radarr(formats: Value, profiles: Value) -> MockServer {
    let server = MockServer::start().await;
    let routes = [
        ("/api", json!({"current": "v3"})),
        (
            "/api/v3/system/status",
            json!({"appName": "Radarr", "version": "6.0.4.10291"}),
        ),
        ("/api/v3/customformat", formats),
        ("/api/v3/qualityprofile", profiles),
    ];
    for (route, body) in routes {
        Mock::given(method("GET"))
            .and(path(route))
            .and(header("X-Api-Key", "synthetic-radarr-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    server
}

fn radarr_connection(server: &MockServer) -> RuleImportConnection {
    RuleImportConnection {
        source: RuleImportSource::Radarr,
        base_url: server.uri(),
        api_key: Some("synthetic-radarr-key".into()),
    }
}

async fn seed_scryer_profiles(ctx: &TestContext, names: &[&str]) {
    seed_typed_settings_definitions(ctx).await;
    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default settings actor");
    let profiles = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let mut profile = scryer_application::builtin_1080p_profile();
            profile.id = format!("sample-profile-{index}");
            profile.name = (*name).to_string();
            profile
        })
        .collect();
    ctx.app
        .save_quality_profile_settings(
            &actor,
            scryer_application::SaveQualityProfileSettings {
                profiles,
                replace_existing: false,
                global_profile_id: None,
                category_selections: vec![],
                global_scoring_persona: None,
                category_persona_selections: vec![],
            },
        )
        .await
        .expect("seed quality profiles");
}

#[tokio::test]
async fn radarr_rule_import_saves_one_disabled_rule_per_format_and_skips_them_on_a_rerun() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        seed_scryer_profiles(&ctx, &["Sample Profile A", "Sample Profile B"]).await;
        let actor = ctx
            .users
            .create(User::new_admin("rule-import-radarr"))
            .await
            .expect("operator");
        let server = radarr(
            json!([
                arr_title_format(1, "Sample Format Alpha"),
                arr_title_format(2, "Sample Format Beta"),
                arr_year_format(3, "Sample Format Gamma"),
                arr_title_format(4, "Sample Format Delta"),
            ]),
            json!([
                {"id": 1, "name": "Sample Profile A", "formatItems": [
                    {"format": 1, "name": "Sample Format Alpha", "score": 25},
                    {"format": 2, "name": "Sample Format Beta", "score": 40},
                    {"format": 3, "name": "Sample Format Gamma", "score": 5},
                    {"format": 4, "name": "Sample Format Delta", "score": 0}
                ]},
                {"id": 2, "name": "Sample Profile B", "formatItems": [
                    {"format": 1, "name": "Sample Format Alpha", "score": 25},
                    {"format": 2, "name": "Sample Format Beta", "score": -10}
                ]}
            ]),
        )
        .await;

        let run = ctx
            .app
            .start_rule_import(&actor, "radarr-first", radarr_connection(&server))
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&actor, &run.id)
            .await
            .expect("settles");
        assert_eq!(report.phase, RuleImportPhase::Finished);
        assert_eq!(report.reason, None);
        let states: Vec<_> = report.items.iter().map(|item| item.state).collect();
        assert_eq!(
            states,
            [
                RuleImportItemState::Saved,
                RuleImportItemState::Saved,
                RuleImportItemState::Failed,
                RuleImportItemState::Saved,
            ]
        );
        assert_eq!(report.counts.saved, 3);
        assert_eq!(report.counts.failed, 1);
        let gamma = &report.items[2];
        assert_eq!(gamma.saved_name, None);
        assert_eq!(gamma.rule_set_id, None);
        assert_eq!(
            gamma.reason.as_deref(),
            Some(rule_import_arr::reasons::FORMAT_UNTRANSLATABLE)
        );
        assert!(!gamma.facts.is_empty());
        assert!(gamma.facts[0].details.as_ref().expect("details")["location"].is_string());
        assert!(
            report.items[3]
                .facts
                .iter()
                .any(|fact| fact.code == rule_import_arr::reasons::FORMAT_UNSCORED)
        );

        let rule_sets = ctx.app.list_rule_sets(&actor).await.expect("rule sets");
        let mut imported: Vec<_> = rule_sets
            .iter()
            .filter(|rule_set| rule_set.name.starts_with("(Radarr) "))
            .collect();
        imported.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<_> = imported
            .iter()
            .map(|rule_set| rule_set.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "(Radarr) Sample Format Alpha",
                "(Radarr) Sample Format Beta",
                "(Radarr) Sample Format Delta",
            ]
        );
        for rule_set in &imported {
            assert!(!rule_set.enabled, "{}", rule_set.name);
            assert_eq!(rule_set.description, "Imported from Radarr 6.0.4.10291.");
            assert_eq!(rule_set.applied_facets, [MediaFacet::Movie]);
            let item = report
                .items
                .iter()
                .find(|item| item.saved_name.as_deref() == Some(rule_set.name.as_str()))
                .expect("item for rule set");
            assert_eq!(item.rule_set_id.as_deref(), Some(rule_set.id.as_str()));
        }
        assert!(imported[0].rego_source.contains(":= 25 if {"));
        assert!(
            imported[1]
                .rego_source
                .contains("\"Sample Profile A\": 40,\n  \"Sample Profile B\": -10\n}"),
            "{}",
            imported[1].rego_source
        );

        let rerun = ctx
            .app
            .start_rule_import(&actor, "radarr-second", radarr_connection(&server))
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&actor, &rerun.id)
            .await
            .expect("settles");
        let states: Vec<_> = report.items.iter().map(|item| item.state).collect();
        assert_eq!(
            states,
            [
                RuleImportItemState::SkippedExists,
                RuleImportItemState::SkippedExists,
                RuleImportItemState::Failed,
                RuleImportItemState::SkippedExists,
            ]
        );
        assert!(
            report
                .items
                .iter()
                .filter(|item| item.state == RuleImportItemState::SkippedExists)
                .all(|item| item.reason.as_deref() == Some(refusal::ALREADY_EXISTS))
        );
        assert_eq!(
            ctx.app
                .list_rule_sets(&actor)
                .await
                .expect("rule sets")
                .len(),
            rule_sets.len()
        );
    })
    .await
    .expect("radarr rule import scenario bound");
}

#[tokio::test]
async fn radarr_rule_import_flags_profiles_scryer_does_not_have_for_review() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        seed_scryer_profiles(&ctx, &["Sample Profile A"]).await;
        let actor = ctx
            .users
            .create(User::new_admin("rule-import-review"))
            .await
            .expect("operator");
        let server = radarr(
            json!([arr_title_format(1, "Sample Format Alpha")]),
            json!([
                {"id": 1, "name": "Sample Profile A", "formatItems": [
                    {"format": 1, "name": "Sample Format Alpha", "score": 25}
                ]},
                {"id": 2, "name": "Sample Profile Unmatched", "formatItems": [
                    {"format": 1, "name": "Sample Format Alpha", "score": -10}
                ]}
            ]),
        )
        .await;
        let run = ctx
            .app
            .start_rule_import(&actor, "radarr-review", radarr_connection(&server))
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&actor, &run.id)
            .await
            .expect("settles");
        let item = &report.items[0];
        assert_eq!(item.state, RuleImportItemState::SavedNeedsReview);
        assert_eq!(
            item.saved_name.as_deref(),
            Some("(Radarr) REVIEW - Sample Format Alpha")
        );
        assert_eq!(
            item.reason.as_deref(),
            Some(rule_import_arr::reasons::PROFILE_SCORES_UNMATCHED)
        );
        let details = item.facts[0].details.as_ref().expect("details");
        assert_eq!(
            details["unmatchedProfiles"],
            json!(["Sample Profile Unmatched"])
        );
        assert_eq!(
            details["scores"],
            json!([
                {"profile": "Sample Profile A", "score": 25},
                {"profile": "Sample Profile Unmatched", "score": -10}
            ])
        );
        assert_eq!(report.counts.saved_needs_review, 1);
        let (stored, _) = ctx
            .app
            .rule_import_status(&actor, &run.id)
            .await
            .expect("status");
        assert_eq!(stored.status, JobRunStatus::Warning);
        let rule_set = ctx
            .app
            .list_rule_sets(&actor)
            .await
            .expect("rule sets")
            .into_iter()
            .find(|rule_set| rule_set.name == "(Radarr) REVIEW - Sample Format Alpha")
            .expect("review rule saved");
        assert!(!rule_set.enabled);
        assert!(rule_set.rego_source.contains(":= 0 if {"));
        assert!(
            rule_set
                .rego_source
                .contains("#   \"Sample Profile Unmatched\": -10")
        );
    })
    .await
    .expect("radarr review scenario bound");
}
