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
async fn rule_import_fetches_purges_the_connection_and_reports_source_facts() {
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
        assert_eq!(report.reason, None);
        // Groups without rules cannot be translated; each is kept as a
        // disabled starter rule.
        let states: Vec<_> = report.items.iter().map(|item| item.state).collect();
        assert_eq!(states, vec![RuleImportItemState::SavedFailedTranslation; 2]);
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
        assert_eq!(stored.status, JobRunStatus::Warning);
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
        assert_eq!(report["status"], "WARNING");
        assert_eq!(report["active"], false);
        assert_eq!(report["source"], "MAINTAINERR");
        assert_eq!(report["phase"], "FINISHED");
        assert_eq!(report["reason"], Value::Null);
        assert_eq!(report["counts"]["total"], 2);
        assert_eq!(report["counts"]["savedFailedTranslation"], 2);
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

const MAINTAINERR_CONSTANTS: &str = include_str!(
    "../../../scryer-application/src/lifecycle_rules/maintainerr/fixtures/live-shape-constants.json"
);

/// One rule row as Maintainerr's `GET /api/rules` serves it.
fn maintainerr_rule(id: i64, group_id: i64, first: (i64, i64), action: i64, with: Value) -> Value {
    let mut dto = json!({
        "operator": null,
        "action": action,
        "firstVal": [first.0, first.1],
        "section": 0,
    });
    if let Value::Object(extra) = with {
        for (key, value) in extra {
            dto[key] = value;
        }
    }
    json!({
        "id": id,
        "ruleJson": dto.to_string(),
        "ruleGroupId": group_id,
        "section": 0,
        "isActive": true,
    })
}

fn maintainerr_group(id: i64, name: &str, arr_action: i64, rules: Vec<Value>) -> Value {
    json!({
        "id": id,
        "name": name,
        "description": "Synthetic description",
        "libraryId": "sample-library",
        "isActive": true,
        "useRules": true,
        "dataType": "movie",
        "rules": rules,
        "collection": {
            "id": id,
            "title": name,
            "arrAction": arr_action,
            "deleteAfterDays": 21,
            "type": "movie",
            "radarrSettingsId": 1,
            "manualCollection": false,
        },
    })
}

/// A Maintainerr with three groups: one that deletes large files, one with
/// no rules (it cannot be translated), and, when `with_person_fact`, one that
/// reads who watched a title.
async fn maintainerr_with_groups(with_person_fact: bool) -> MockServer {
    // Delete, and unmonitor, as Maintainerr numbers its actions.
    const DELETE: i64 = 0;
    const UNMONITOR: i64 = 3;
    let mut groups = vec![
        maintainerr_group(
            1,
            "Sample Rule Alpha",
            DELETE,
            vec![maintainerr_rule(
                101,
                1,
                (1, 7),
                0,
                json!({"customVal": {"ruleTypeId": 0, "value": "100"}}),
            )],
        ),
        maintainerr_group(2, "Sample Rule Bravo", UNMONITOR, Vec::new()),
    ];
    if with_person_fact {
        groups.push(maintainerr_group(
            3,
            "Sample Rule Charlie",
            UNMONITOR,
            vec![maintainerr_rule(
                301,
                3,
                (6, 1),
                4,
                json!({"customVal": {"ruleTypeId": 2, "value": "sample-user"}}),
            )],
        ));
    }
    let server = MockServer::start().await;
    let constants: Value = serde_json::from_str(MAINTAINERR_CONSTANTS).expect("constants");
    let routes = [
        (
            "/api/settings/version",
            ResponseTemplate::new(200).set_body_string("2.19.0"),
        ),
        (
            "/api/rules",
            ResponseTemplate::new(200).set_body_json(Value::from(groups)),
        ),
        (
            "/api/rules/constants",
            ResponseTemplate::new(200).set_body_json(constants),
        ),
        (
            "/api/rules/users",
            ResponseTemplate::new(200).set_body_json(json!(["sample-user"])),
        ),
        (
            "/api/servarr/radarr/1/profiles",
            ResponseTemplate::new(200).set_body_json(json!([])),
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

fn item_states(
    report: &scryer_application::rule_import::RuleImportProgress,
) -> Vec<(&str, RuleImportItemState)> {
    report
        .items
        .iter()
        .map(|item| (item.source_name.as_str(), item.state))
        .collect()
}

#[tokio::test]
async fn maintainerr_rule_import_saves_disabled_lifecycle_rules_and_skips_them_on_a_rerun() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        let actor = ctx
            .users
            .create(User::new_admin("sample-user"))
            .await
            .expect("operator");
        let server = maintainerr_with_groups(true).await;
        let run = ctx
            .app
            .start_rule_import(&actor, "maintainerr-save", maintainerr_connection(&server))
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&actor, &run.id)
            .await
            .expect("settles");
        assert_eq!(report.reason, None, "{report:?}");
        assert_eq!(
            item_states(&report),
            vec![
                ("Sample Rule Alpha", RuleImportItemState::Saved),
                (
                    "Sample Rule Bravo",
                    RuleImportItemState::SavedFailedTranslation
                ),
                ("Sample Rule Charlie", RuleImportItemState::Saved),
            ]
        );
        assert_eq!(report.items[0].key, "rulegroup:1");
        assert_eq!(
            report.items[1].reason.as_deref(),
            Some(scryer_application::rule_import_maintainerr::reasons::GROUP_UNTRANSLATABLE)
        );
        let empty = report.items[1]
            .facts
            .iter()
            .find(|fact| fact.code == "maintainerr.group.rules_empty")
            .expect("the untranslated group says why");
        assert_eq!(
            empty.details.as_ref().expect("details")["severity"],
            "ERROR"
        );
        let (stored, _) = ctx
            .app
            .rule_import_status(&actor, &run.id)
            .await
            .expect("status");
        assert_eq!(stored.status, JobRunStatus::Warning);

        let mut saved = ctx
            .app
            .list_lifecycle_rule_sets(&actor)
            .await
            .expect("lifecycle rules");
        saved.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<_> = saved.iter().map(|rule| rule.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "(Maintainerr) FAILED - Sample Rule Bravo",
                "(Maintainerr) Sample Rule Alpha",
                "(Maintainerr) Sample Rule Charlie",
            ]
        );
        for rule in &saved {
            assert_eq!(rule.state, scryer_domain::LifecycleRuleState::Disabled);
            assert!(rule.library_ids.is_empty(), "every library");
            assert_eq!(
                rule.schedule,
                scryer_domain::default_lifecycle_rule_schedule()
            );
            let item = report
                .items
                .iter()
                .find(|item| item.saved_name.as_deref() == Some(rule.name.as_str()))
                .expect("the report names the saved rule");
            assert_eq!(item.rule_set_id.as_deref(), Some(rule.id.as_str()));
            let detail = ctx
                .app
                .get_lifecycle_rule_set(&actor, &rule.id)
                .await
                .expect("read")
                .expect("exists");
            assert_eq!(detail.revision.grace_days, 21);
        }
        // Only the rule that deletes carries a per-run maximum.
        assert_eq!(saved[1].max_titles_per_run, Some(10));
        assert_eq!(saved[0].max_titles_per_run, None);
        assert_eq!(saved[2].max_titles_per_run, None);

        let rerun = ctx
            .app
            .start_rule_import(&actor, "maintainerr-rerun", maintainerr_connection(&server))
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&actor, &rerun.id)
            .await
            .expect("settles");
        assert!(
            report
                .items
                .iter()
                .all(|item| item.state == RuleImportItemState::SkippedExists
                    && item.reason.as_deref() == Some(refusal::ALREADY_EXISTS)),
            "{report:?}"
        );
        let (stored, _) = ctx
            .app
            .rule_import_status(&actor, &rerun.id)
            .await
            .expect("status");
        assert_eq!(stored.status, JobRunStatus::Completed);
        assert_eq!(
            ctx.app
                .list_lifecycle_rule_sets(&actor)
                .await
                .expect("lifecycle rules")
                .len(),
            3
        );
    })
    .await
    .expect("maintainerr save scenario bound");
}

#[tokio::test]
async fn maintainerr_rule_reading_who_watched_needs_permission_management() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        let admin = ctx
            .users
            .create(User::new_admin("rule-import-admin"))
            .await
            .expect("admin");
        let catalog = ctx
            .app
            .create_user(
                &admin,
                "sample-user".to_string(),
                "catalog-pass1".to_string(),
                AppPermissionMask::from_permission(
                    scryer_domain::AppPermission::ManageCatalogSettings,
                ),
                vec![],
            )
            .await
            .expect("catalog user");
        let server = maintainerr_with_groups(true).await;
        let run = ctx
            .app
            .start_rule_import(&catalog, "person-fact", maintainerr_connection(&server))
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&catalog, &run.id)
            .await
            .expect("settles");
        assert_eq!(
            item_states(&report),
            vec![
                ("Sample Rule Alpha", RuleImportItemState::Saved),
                (
                    "Sample Rule Bravo",
                    RuleImportItemState::SavedFailedTranslation
                ),
                ("Sample Rule Charlie", RuleImportItemState::Failed),
            ]
        );
        let refused = &report.items[2];
        let code =
            scryer_application::error_reasons::lifecycle::PERSON_TARGETING_PERMISSION_REQUIRED;
        assert_eq!(refused.reason.as_deref(), Some(code));
        assert!(refused.facts.iter().any(|fact| fact.code == code));
        assert_eq!(refused.rule_set_id, None);
        assert_eq!(
            ctx.app
                .list_lifecycle_rule_sets(&catalog)
                .await
                .expect("lifecycle rules")
                .len(),
            2
        );

        // An administrator saves the refused group; the others already exist.
        let run = ctx
            .app
            .start_rule_import(&admin, "person-fact-admin", maintainerr_connection(&server))
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&admin, &run.id)
            .await
            .expect("settles");
        assert_eq!(
            item_states(&report),
            vec![
                ("Sample Rule Alpha", RuleImportItemState::SkippedExists),
                ("Sample Rule Bravo", RuleImportItemState::SkippedExists),
                ("Sample Rule Charlie", RuleImportItemState::Saved),
            ]
        );
    })
    .await
    .expect("maintainerr person fact scenario bound");
}

#[tokio::test]
async fn an_unreachable_maintainerr_fails_the_run_with_a_code_and_saves_nothing() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        let actor = ctx
            .users
            .create(User::new_admin("rule-import-unreachable"))
            .await
            .expect("operator");
        let server = MockServer::start().await;
        let base_url = server.uri();
        drop(server);
        let run = ctx
            .app
            .start_rule_import(
                &actor,
                "maintainerr-unreachable",
                RuleImportConnection {
                    source: RuleImportSource::Maintainerr,
                    base_url,
                    api_key: None,
                },
            )
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&actor, &run.id)
            .await
            .expect("settles");
        assert_eq!(report.phase, RuleImportPhase::Finished);
        let reason = report.reason.as_deref().expect("a run reason");
        assert!(
            scryer_application::error_reasons::external_import::ALL.contains(&reason),
            "{reason}"
        );
        assert!(report.items.is_empty());
        let (stored, _) = ctx
            .app
            .rule_import_status(&actor, &run.id)
            .await
            .expect("status");
        assert_eq!(stored.status, JobRunStatus::Failed);
        assert!(
            ctx.app
                .list_lifecycle_rule_sets(&actor)
                .await
                .expect("lifecycle rules")
                .is_empty()
        );
    })
    .await
    .expect("maintainerr unreachable scenario bound");
}

#[tokio::test]
async fn overlapping_radarr_rule_imports_save_each_format_once() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        seed_scryer_profiles(&ctx, &["Sample Profile A"]).await;
        let actor = ctx
            .users
            .create(User::new_admin("rule-import-overlap"))
            .await
            .expect("operator");
        let server = radarr(
            json!([
                arr_title_format(1, "Sample Format Alpha"),
                arr_title_format(2, "Sample Format Beta"),
            ]),
            json!([
                {"id": 1, "name": "Sample Profile A", "formatItems": [
                    {"format": 1, "name": "Sample Format Alpha", "score": 25},
                    {"format": 2, "name": "Sample Format Beta", "score": 10}
                ]}
            ]),
        )
        .await;

        // Both runs are accepted before either settles, so their plans can
        // both see no existing rule; the save must still write each once.
        let first = ctx
            .app
            .start_rule_import(&actor, "radarr-overlap-a", radarr_connection(&server))
            .await
            .expect("first accepted");
        let second = ctx
            .app
            .start_rule_import(&actor, "radarr-overlap-b", radarr_connection(&server))
            .await
            .expect("second accepted");
        assert_ne!(first.id, second.id);
        let (first, second) = tokio::join!(
            ctx.app.wait_rule_import(&actor, &first.id),
            ctx.app.wait_rule_import(&actor, &second.id),
        );
        let (first, second) = (
            first.expect("first settles"),
            second.expect("second settles"),
        );

        for index in 0..2 {
            let states = [first.items[index].state, second.items[index].state];
            assert!(
                states.contains(&RuleImportItemState::Saved)
                    && states.contains(&RuleImportItemState::SkippedExists),
                "{states:?}"
            );
            let skipped = if first.items[index].state == RuleImportItemState::SkippedExists {
                &first.items[index]
            } else {
                &second.items[index]
            };
            assert_eq!(skipped.reason.as_deref(), Some(refusal::ALREADY_EXISTS));
            assert_eq!(skipped.rule_set_id, None);
        }

        let mut names: Vec<_> = ctx
            .app
            .list_rule_sets(&actor)
            .await
            .expect("rule sets")
            .into_iter()
            .map(|rule_set| rule_set.name)
            .filter(|name| name.starts_with("(Radarr) "))
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "(Radarr) Sample Format Alpha",
                "(Radarr) Sample Format Beta"
            ]
        );
    })
    .await
    .expect("overlapping rule import scenario bound");
}

#[tokio::test]
async fn a_radarr_response_over_the_rule_import_cap_fails_the_run_with_a_code() {
    timeout(FAILURE_BOUND, async {
        let ctx = TestContext::new().await;
        seed_scryer_profiles(&ctx, &["Sample Profile A"]).await;
        let actor = ctx
            .users
            .create(User::new_admin("rule-import-oversized"))
            .await
            .expect("operator");
        let oversized =
            "x".repeat(scryer_application::external_import::RULE_IMPORT_ARR_MAX_RESPONSE_BYTES);
        let server = radarr(json!([oversized]), json!([])).await;
        let run = ctx
            .app
            .start_rule_import(&actor, "radarr-oversized", radarr_connection(&server))
            .await
            .expect("accepted");
        let report = ctx
            .app
            .wait_rule_import(&actor, &run.id)
            .await
            .expect("settles");
        assert_eq!(
            report.reason.as_deref(),
            Some(scryer_application::error_reasons::external_import::SOURCE_RESPONSE_TOO_LARGE)
        );
        assert!(report.items.is_empty());
        let (stored, _) = ctx
            .app
            .rule_import_status(&actor, &run.id)
            .await
            .expect("status");
        assert_eq!(stored.status, JobRunStatus::Failed);
    })
    .await
    .expect("oversized rule import scenario bound");
}
