#![recursion_limit = "256"]

//! End-to-end coverage for the request-rule GraphQL surface (spec 0003 §7).
//!
//! Three properties are what these tests exist to hold:
//!
//! 1. **The requester and the approver see different things.** A pre-flight has
//!    no vote field at all, and a stored decision read back by its own requester
//!    comes back with `votes` emptied and its `reasons` intact (FR-020).
//! 2. **Arming is three deliberate steps.** A rule is created disabled, armed by
//!    a second call, and the instance gate is a third — and each step is gated
//!    on a different authority.
//! 3. **A verdict reaches the request.** With the gate on and a rule in
//!    `ENFORCE`, a submitted request comes back approved or rejected, the title
//!    carries the policy tags, the lease is a real claim, and the trace explains
//!    all of it.

mod common;

use async_graphql::Variables;
use common::TestContext;
use scryer_application::UserRepository;
use scryer_domain::{
    AppPermission, AppPermissionMask, Id, LibraryPermission, LibraryPermissionMask, MediaFacet,
    User, UserAuthorization,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, Request, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Execute over HTTP as the instance's default administrator.
async fn gql(ctx: &TestContext, query: &str, variables: Value) -> Value {
    let client = ctx.http_client();
    let resp = client
        .post(ctx.graphql_url())
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("should be valid JSON")
}

/// Execute against the schema directly so the test can choose the actor.
async fn gql_as(ctx: &TestContext, query: &str, variables: Value, actor: &User) -> Value {
    let request = async_graphql::Request::new(query)
        .variables(Variables::from_json(variables))
        .data(actor.clone());
    let response = ctx.schema.execute(request).await;
    serde_json::to_value(&response).expect("serialize gql response")
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

fn error_messages(body: &Value) -> String {
    body["errors"]
        .as_array()
        .map(|errors| {
            errors
                .iter()
                .filter_map(|error| error["message"].as_str())
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default()
}

fn assert_error_contains(body: &Value, expected: &str) {
    let messages = error_messages(body);
    assert!(
        !messages.is_empty(),
        "expected GraphQL errors containing {expected:?}: {body}"
    );
    assert!(
        messages.contains(expected),
        "expected GraphQL error containing {expected:?}, got {messages}"
    );
}

// ── Matchers ────────────────────────────────────────────────────────────────
//
// The package declaration is deliberately absent from all of these: the server
// owns it, and the editor never sees it.

/// Approves every manually submitted request and tags what it approved.
const APPROVE_EVERYTHING: &str = "approve if {\n\
     \tinput.request.origin == \"manual\"\n\
     }\n\
     \n\
     tags contains \"auto-approved\" if {\n\
     \tinput.request.origin == \"manual\"\n\
     }\n";

/// Denies every manually submitted request, with a reason an approver — and the
/// requester — can read.
const DENY_EVERYTHING: &str = "deny if {\n\
     \tinput.request.origin == \"manual\"\n\
     }\n\
     \n\
     reasons contains \"policy_denied\" if {\n\
     \tinput.request.origin == \"manual\"\n\
     }\n";

/// Reads the requester document, so authoring it needs permission-management
/// authority on top of catalog settings.
const PERSON_TARGETED: &str = "approve if {\n\
     \tinput.requester.username == \"operator\"\n\
     }\n";

/// Names a fact that does not exist.
const UNKNOWN_FACT: &str = "approve if {\n\
     \tinput.facts.not_a_fact\n\
     }\n";

const RULE_SET_FIELDS: &str = r#"
    id
    name
    description
    enabled
    evaluationMode
    libraryIds
    currentRevisionNumber
    decisionCount
    createdAt
    updatedAt
"#;

fn detail_fields() -> String {
    format!(
        "ruleSet {{{RULE_SET_FIELDS}}}
         revision {{
             id
             ruleSetId
             revisionNumber
             regoSource
             matcherContentHash
             createdBy
             createdAt
         }}"
    )
}

fn create_mutation() -> String {
    format!(
        "mutation($input: CreateRequestRuleSetInput!) {{
            createRequestRuleSet(input: $input) {{{}}}
        }}",
        detail_fields()
    )
}

fn set_mode_mutation() -> String {
    format!(
        "mutation($input: SetRequestRuleModeInput!) {{
            setRequestRuleMode(input: $input) {{{}}}
        }}",
        detail_fields()
    )
}

const DECISION_FIELDS: &str = r#"
    id
    requestId
    evaluatedAt
    mode
    effectiveOutcome
    policyOutcome
    fallbackReason
    votes { ruleSetId ruleSetName revisionNumber vote held reasonCodes tags error }
    reasons { code ruleName }
    tags
    inputSchemaVersion
"#;

fn preview_mutation() -> String {
    format!(
        "mutation($input: PreviewRequestRuleInput!) {{
            previewRequestRule(input: $input) {{
                ruleSetId
                matcherContentHash
                metadataPartial
                inputDocument
                decision {{{DECISION_FIELDS}}}
            }}
        }}"
    )
}

const CLAIM_FIELDS: &str = r#"
    id
    titleId
    libraryId
    producer
    producerRef
    kind
    state
    durationDays
    startsAt
    expiresAt
    releasedReason
"#;

fn my_requests_query() -> String {
    format!(
        "query {{
            myMediaRequests {{
                id
                status
                createdTitleId
                requestedLeaseDays
                approvedLeaseDays
                policyTags
                lease {{ requestedDays approvedDays state startsAt expiresAt }}
                decision {{{DECISION_FIELDS}}}
                metadata {{
                    partial
                    missing
                    genres
                    ageRating
                    certificationLabel
                    certificationRank
                    isAdult
                    awardCount
                    contentRatings {{ country ageRating certifications {{ value source }} }}
                }}
            }}
        }}"
    )
}

fn movie_library_id() -> String {
    scryer_domain::default_library_id_for_facet(&MediaFacet::Movie)
}

/// The draft every submit and pre-flight in this file uses. TVDB 123456 is the
/// movie the shared metadata mock resolves.
fn submit_input(lease_days: Option<i32>) -> Value {
    let mut input = json!({
        "libraryId": movie_library_id(),
        "facet": "MOVIE",
        "title": "Test Movie Title",
        "year": 2024,
        "externalIds": [
            // The SMG id short-circuits the resolve step, so one metadata mock
            // answers the whole enrichment.
            { "source": "smg", "value": "101" },
            { "source": "tvdb", "value": "123456" },
            { "source": "imdb", "value": "tt1234567" },
        ],
    });
    if let Some(days) = lease_days {
        input["requestedLeaseDays"] = json!(days);
    }
    input
}

/// A stored user carrying exactly the authority the test needs.
///
/// The row is created so the request join tables have something to point at;
/// the returned actor carries a `loaded` authorization so `gql_as` decides
/// against the mask the test asked for rather than whatever the store echoes.
async fn user_with(
    ctx: &TestContext,
    username: &str,
    app: AppPermissionMask,
    library: LibraryPermissionMask,
) -> User {
    let authorization = UserAuthorization {
        app,
        libraries: [(movie_library_id(), library)].into_iter().collect(),
        default_library: LibraryPermissionMask::NONE,
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        login_status: Default::default(),
        loaded: true,
    };
    let stored = ctx
        .users
        .create(User {
            id: Id::new().0,
            username: username.to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: authorization.clone(),
        })
        .await
        .expect("user should create");
    User {
        authorization,
        ..stored
    }
}

async fn requester(ctx: &TestContext, username: &str) -> User {
    user_with(
        ctx,
        username,
        AppPermissionMask::NONE,
        LibraryPermissionMask::from_permissions([
            LibraryPermission::View,
            LibraryPermission::Request,
        ]),
    )
    .await
}

/// Create a rule set as the default administrator and return the rule-set ID.
async fn create_rule(ctx: &TestContext, name: &str, rego: &str) -> String {
    let body = gql(
        ctx,
        &create_mutation(),
        json!({ "input": { "name": name, "regoSource": rego } }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["createRequestRuleSet"]["ruleSet"]["id"]
        .as_str()
        .expect("rule set id")
        .to_string()
}

async fn set_mode(ctx: &TestContext, rule_set_id: &str, mode: &str) {
    let body = gql(
        ctx,
        &set_mode_mutation(),
        json!({ "input": { "ruleSetId": rule_set_id, "mode": mode } }),
    )
    .await;
    assert_no_errors(&body);
}

/// Answer the title-id metadata query the request enrichment actually makes.
///
/// The shared harness answers every metadata query with one `metadataBulk`
/// fixture, which is what catalog hydration asks for. Request enrichment asks
/// `titles(ids:)` instead, so without this the snapshot comes back wholly
/// unavailable and every metadata fact reads as unknown. The payload is the
/// same fixture movie with a US `PG-13` certification, so the certification
/// facts have something real to rank.
fn is_title_id_query(request: &Request) -> bool {
    let names_titles =
        |value: &str| value.contains("query Titles(") || value.contains("titles(ids");
    let body = request.body_json::<Value>().ok().is_some_and(|body| {
        body.get("operationName")
            .and_then(Value::as_str)
            .is_some_and(|operation| operation == "Titles")
            || body
                .get("query")
                .and_then(Value::as_str)
                .is_some_and(names_titles)
    });
    let query = request.url.query_pairs().any(|(key, value)| {
        (key == "operationName" && value == "Titles") || (key == "query" && names_titles(&value))
    });
    body || query
}

async fn mount_title_id_metadata(ctx: &TestContext) {
    let fixture = json!({
        "data": {
            "titles": {
                "movies": [{
                    "id": 101,
                    "kind": "movie",
                    "primary_source": "tvdb",
                    "tvdb_id": 123456,
                    "name": "Test Movie Title",
                    "slug": "test-movie-title",
                    "type": "movie",
                    "year": 2024,
                    "status": "Released",
                    "overview": "A gripping tale of testing integration.",
                    "poster_url": "https://artworks.thetvdb.com/banners/movies/123456/posters/test.jpg",
                    "language": "eng",
                    "original_language": "eng",
                    "runtime_minutes": 142,
                    "sort_title": "Test Movie Title",
                    "imdb_id": "tt1234567",
                    "tmdb_id": 654321,
                    "tmdb_popularity": 12.5,
                    "tmdb_vote_average": 7.25,
                    "tmdb_vote_count": 4321,
                    "anidb_id": null,
                    "genres": ["Action", "Thriller"],
                    "content_ratings": [{
                        "country": "usa",
                        "certifications": [
                            { "value": "PG-13", "source": "mpaa", "release_type": 3 }
                        ],
                        "age_rating": 13,
                        "age_rating_source": "mpaa"
                    }],
                    "mdblist": null,
                    "awards": [],
                    "canonical_tags": [
                        {
                            "key": "canonical:genre:action",
                            "category": "genre",
                            "name": "Action",
                            "confidence": 1.0,
                            "sources": [],
                            "source_tag_keys": [],
                            "is_adult": false,
                            "is_spoiler": false
                        }
                    ],
                    "external_ids": [],
                    "studio": "Test Studios",
                    "tmdb_release_date": "2024-06-15",
                    "rating": null,
                    "rating_sources": [],
                    "external_ratings": [],
                    "credits": [],
                    "artworks": []
                }],
                "missing_ids": [],
                "redirects": []
            }
        }
    })
    .to_string();

    for verb in ["GET", "POST"] {
        Mock::given(method(verb))
            .and(path("/graphql"))
            .and(is_title_id_query)
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
            .with_priority(1)
            .mount(&ctx.smg_server)
            .await;
    }
}

/// Register the gate definition the way service bootstrap does.
///
/// The harness builds its stores directly rather than running bootstrap, and
/// the settings store refuses a write against an unregistered key. The
/// production seed lives beside the maintenance gates in
/// `crates/scryer/src/settings_bootstrap.rs`.
async fn seed_request_rule_gate_definition(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            scryer_infrastructure_sql::types::SettingDefinitionSeed {
                category: "general".into(),
                scope: "system".into(),
                key_name: scryer_application::REQUEST_RULE_GATE_EVALUATION_KEY.into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("seed the request-rule gate definition");
}

async fn arm_gate(ctx: &TestContext, enabled: bool) {
    seed_request_rule_gate_definition(ctx).await;
    let body = gql(
        ctx,
        "mutation($input: SetRequestRuleInstanceGatesInput!) {
            setRequestRuleInstanceGates(input: $input) { evaluationEnabled }
        }",
        json!({ "input": { "evaluationEnabled": enabled } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["setRequestRuleInstanceGates"]["evaluationEnabled"],
        json!(enabled)
    );
}

/// Arm one rule in `mode` with the instance gate on, then submit one request as
/// `actor`, returning that requester's view of their own request.
async fn submit_under(
    ctx: &TestContext,
    actor: &User,
    rego: &str,
    mode: &str,
    lease_days: Option<i32>,
) -> Value {
    mount_title_id_metadata(ctx).await;
    let rule_set_id = create_rule(ctx, "Policy under test", rego).await;
    set_mode(ctx, &rule_set_id, mode).await;
    arm_gate(ctx, true).await;

    let submitted = gql_as(
        ctx,
        "mutation($input: SubmitMediaRequestInput!) {
            submitMediaRequest(input: $input) { requestId }
        }",
        json!({ "input": submit_input(lease_days) }),
        actor,
    )
    .await;
    assert_no_errors(&submitted);

    let mine = gql_as(ctx, &my_requests_query(), json!({}), actor).await;
    assert_no_errors(&mine);
    mine["data"]["myMediaRequests"]
        .as_array()
        .and_then(|requests| requests.first())
        .cloned()
        .unwrap_or_else(|| panic!("the requester should see their own request: {mine}"))
}

// ===========================================================================
// 1. Authoring round trip
// ===========================================================================

#[tokio::test]
async fn request_rule_create_list_get_and_revisions_round_trip() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        &create_mutation(),
        json!({
            "input": {
                "name": "Approve manual requests",
                "description": "Everything a person asked for",
                "regoSource": APPROVE_EVERYTHING,
                "libraryIds": [movie_library_id()],
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let detail = &body["data"]["createRequestRuleSet"];
    let rule_set = &detail["ruleSet"];
    let id = rule_set["id"].as_str().expect("rule set id").to_string();
    assert_eq!(rule_set["name"], "Approve manual requests");
    assert_eq!(rule_set["description"], "Everything a person asked for");
    // Created disabled: arming is a second, deliberate call.
    assert_eq!(rule_set["enabled"], false);
    assert_eq!(rule_set["evaluationMode"], "DISABLED");
    assert_eq!(rule_set["libraryIds"], json!([movie_library_id()]));
    assert_eq!(rule_set["currentRevisionNumber"], 1);
    assert_eq!(rule_set["decisionCount"], 0);

    let revision = &detail["revision"];
    assert_eq!(revision["revisionNumber"], 1);
    assert_eq!(revision["ruleSetId"], json!(id));
    assert!(
        revision["matcherContentHash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())
    );
    let source = revision["regoSource"].as_str().expect("rego source");
    assert!(
        source.contains("input.request.origin"),
        "the editor gets its own source back: {source}"
    );
    assert!(
        !source.contains("package "),
        "the server-owned package declaration is stripped: {source}"
    );

    // Editing the matcher appends revision two and leaves revision one alone.
    let updated = gql(
        &ctx,
        &format!(
            "mutation($input: UpdateRequestRuleMatcherInput!) {{
                updateRequestRuleMatcher(input: $input) {{{}}}
            }}",
            detail_fields()
        ),
        json!({ "input": { "ruleSetId": id, "regoSource": DENY_EVERYTHING } }),
    )
    .await;
    assert_no_errors(&updated);
    assert_eq!(
        updated["data"]["updateRequestRuleMatcher"]["revision"]["revisionNumber"],
        2
    );

    let revisions = gql(
        &ctx,
        "query($ruleSetId: ID!) {
            requestRuleRevisions(ruleSetId: $ruleSetId) { revisionNumber regoSource }
        }",
        json!({ "ruleSetId": id }),
    )
    .await;
    assert_no_errors(&revisions);
    let rows = revisions["data"]["requestRuleRevisions"]
        .as_array()
        .expect("revisions");
    assert_eq!(rows.len(), 2, "the edit appended rather than replaced");

    // Renaming touches no matcher, so no revision is created.
    let renamed = gql(
        &ctx,
        &format!(
            "mutation($input: UpdateRequestRuleMetadataInput!) {{
                updateRequestRuleMetadata(input: $input) {{{RULE_SET_FIELDS}}}
            }}"
        ),
        json!({ "input": { "ruleSetId": id, "name": "Renamed", "libraryIds": [] } }),
    )
    .await;
    assert_no_errors(&renamed);
    assert_eq!(
        renamed["data"]["updateRequestRuleMetadata"]["name"],
        "Renamed"
    );
    assert_eq!(
        renamed["data"]["updateRequestRuleMetadata"]["currentRevisionNumber"],
        2
    );

    let listed = gql(
        &ctx,
        &format!("query {{ requestRuleSets {{{RULE_SET_FIELDS}}} }}"),
        json!({}),
    )
    .await;
    assert_no_errors(&listed);
    assert_eq!(
        listed["data"]["requestRuleSets"]
            .as_array()
            .expect("rule sets")
            .len(),
        1
    );

    let fetched = gql(
        &ctx,
        &format!(
            "query($id: ID!) {{ requestRuleSet(id: $id) {{{}}} }}",
            detail_fields()
        ),
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&fetched);
    assert_eq!(
        fetched["data"]["requestRuleSet"]["ruleSet"]["id"],
        json!(id)
    );

    let deleted = gql(
        &ctx,
        "mutation($id: ID!) { deleteRequestRuleSet(id: $id) { id } }",
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&deleted);
    assert_eq!(deleted["data"]["deleteRequestRuleSet"]["id"], json!(id));

    let gone = gql(
        &ctx,
        &format!(
            "query($id: ID!) {{ requestRuleSet(id: $id) {{{}}} }}",
            detail_fields()
        ),
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&gone);
    assert!(gone["data"]["requestRuleSet"].is_null());
}

// ===========================================================================
// 2. Validation names the reference
// ===========================================================================

#[tokio::test]
async fn validating_an_unknown_fact_path_names_the_rules_context_reference() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        "mutation($input: ValidateRequestRuleInput!) {
            validateRequestRule(input: $input) { valid errors }
        }",
        json!({ "input": { "regoSource": UNKNOWN_FACT } }),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["validateRequestRule"];
    assert_eq!(payload["valid"], false);
    let errors = payload["errors"].as_array().expect("errors").iter().fold(
        String::new(),
        |mut all, error| {
            all.push_str(error.as_str().unwrap_or_default());
            all.push(' ');
            all
        },
    );
    assert!(
        errors.contains("Unknown rule input path") && errors.contains("Rules Context Reference"),
        "an unknown path should point the author at the reference: {errors}"
    );

    // A valid matcher validates, and stores nothing.
    let valid = gql(
        &ctx,
        "mutation($input: ValidateRequestRuleInput!) {
            validateRequestRule(input: $input) { valid errors }
        }",
        json!({ "input": { "regoSource": APPROVE_EVERYTHING } }),
    )
    .await;
    assert_no_errors(&valid);
    assert_eq!(valid["data"]["validateRequestRule"]["valid"], true);

    let listed = gql(
        &ctx,
        &format!("query {{ requestRuleSets {{{RULE_SET_FIELDS}}} }}"),
        json!({}),
    )
    .await;
    assert_no_errors(&listed);
    assert!(
        listed["data"]["requestRuleSets"]
            .as_array()
            .expect("rule sets")
            .is_empty(),
        "validation must persist nothing"
    );
}

// ===========================================================================
// 3. Person-targeting needs permission authority
// ===========================================================================

#[tokio::test]
async fn a_person_targeting_matcher_needs_permission_management_authority() {
    let ctx = TestContext::new().await;

    let catalog_only = user_with(
        &ctx,
        "catalog-only",
        AppPermissionMask::from_permissions([AppPermission::ManageCatalogSettings]),
        LibraryPermissionMask::NONE,
    )
    .await;
    let permissions_admin = user_with(
        &ctx,
        "permissions-admin",
        AppPermissionMask::from_permissions([
            AppPermission::ManageCatalogSettings,
            AppPermission::ManagePermissions,
        ]),
        LibraryPermissionMask::NONE,
    )
    .await;

    let refused = gql_as(
        &ctx,
        &create_mutation(),
        json!({ "input": { "name": "About one person", "regoSource": PERSON_TARGETED } }),
        &catalog_only,
    )
    .await;
    assert!(
        refused.get("errors").is_some(),
        "a catalog-only administrator must not author a rule about a named person: {refused}"
    );

    // The same author may still write a rule about media alone.
    let content_only = gql_as(
        &ctx,
        &create_mutation(),
        json!({ "input": { "name": "About media", "regoSource": APPROVE_EVERYTHING } }),
        &catalog_only,
    )
    .await;
    assert_no_errors(&content_only);

    let accepted = gql_as(
        &ctx,
        &create_mutation(),
        json!({ "input": { "name": "About one person", "regoSource": PERSON_TARGETED } }),
        &permissions_admin,
    )
    .await;
    assert_no_errors(&accepted);
}

// ===========================================================================
// 4. Author preview
// ===========================================================================

#[tokio::test]
async fn the_author_preview_returns_the_vote_the_reasons_the_tags_and_the_input_document() {
    let ctx = TestContext::new().await;
    let sample_user = requester(&ctx, "preview-subject").await;
    let rule_set_id = create_rule(&ctx, "Approve everything", APPROVE_EVERYTHING).await;

    let sample = json!({
        "userId": sample_user.id,
        "libraryId": movie_library_id(),
        "externalIds": [{ "source": "tvdb", "value": "123456" }],
        "leaseDays": 30,
    });

    let stored = gql(
        &ctx,
        &preview_mutation(),
        json!({ "input": { "ruleSetId": rule_set_id, "sample": sample } }),
    )
    .await;
    assert_no_errors(&stored);
    let payload = &stored["data"]["previewRequestRule"];
    assert_eq!(payload["ruleSetId"], json!(rule_set_id));
    let decision = &payload["decision"];
    assert!(
        decision["id"].is_null() && decision["requestId"].is_null(),
        "a preview persists no trace to point at: {decision}"
    );
    assert_eq!(decision["policyOutcome"], "AUTO_APPROVE");
    assert_eq!(decision["effectiveOutcome"], "AUTO_APPROVE");
    assert_eq!(decision["votes"][0]["vote"], "APPROVE");
    assert_eq!(decision["tags"], json!(["auto-approved"]));
    // The author-side preview is the one place the whole document is returned.
    let document = &payload["inputDocument"];
    assert!(
        document["facts"].is_object() && document["request"].is_object(),
        "the author should see the exact document the rule saw: {document}"
    );
    assert_eq!(document["request"]["lease_days"], 30);

    // An unsaved draft previews the same way and stores nothing.
    let inline = gql(
        &ctx,
        &preview_mutation(),
        json!({ "input": { "regoSource": DENY_EVERYTHING, "sample": sample } }),
    )
    .await;
    assert_no_errors(&inline);
    let inline_decision = &inline["data"]["previewRequestRule"]["decision"];
    assert_eq!(inline_decision["policyOutcome"], "DENY");
    assert_eq!(inline_decision["mode"], "DISABLED");
    assert_eq!(
        inline_decision["reasons"][0]["code"],
        json!("policy_denied")
    );

    let listed = gql(
        &ctx,
        &format!("query {{ requestRuleSets {{{RULE_SET_FIELDS}}} }}"),
        json!({}),
    )
    .await;
    assert_no_errors(&listed);
    assert_eq!(
        listed["data"]["requestRuleSets"]
            .as_array()
            .expect("rule sets")
            .len(),
        1,
        "an inline preview must not store a rule set"
    );

    // Naming both a stored rule and a draft is ambiguous, and refused.
    let ambiguous = gql(
        &ctx,
        &preview_mutation(),
        json!({ "input": {
            "ruleSetId": rule_set_id,
            "regoSource": DENY_EVERYTHING,
            "sample": sample,
        }}),
    )
    .await;
    assert_error_contains(&ambiguous, "not both");
}

// ===========================================================================
// 5. Requester pre-flight
// ===========================================================================

#[tokio::test]
async fn the_requester_preflight_returns_an_outcome_without_any_rule_internals() {
    let ctx = TestContext::new().await;
    let requester = requester(&ctx, "preflight-requester").await;

    let rule_set_id = create_rule(&ctx, "Deny everything", DENY_EVERYTHING).await;
    set_mode(&ctx, &rule_set_id, "ENFORCE").await;
    arm_gate(&ctx, true).await;

    let body = gql_as(
        &ctx,
        "query($input: SubmitMediaRequestInput!) {
            previewMyRequestDecision(input: $input) {
                outcome
                reasons { code ruleName }
                tags
                metadataPartial
                evaluationMode
            }
        }",
        json!({ "input": submit_input(Some(30)) }),
        &requester,
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["previewMyRequestDecision"];
    assert_eq!(payload["outcome"], "DENY");
    assert_eq!(payload["evaluationMode"], "ENFORCE");
    assert_eq!(payload["reasons"][0]["code"], json!("policy_denied"));
    assert_eq!(payload["reasons"][0]["ruleName"], json!("Deny everything"));

    // The type itself has no vote field: asking for one is a schema error, which
    // is the strongest form the redaction can take.
    let probing = gql_as(
        &ctx,
        "query($input: SubmitMediaRequestInput!) {
            previewMyRequestDecision(input: $input) { outcome votes { vote } }
        }",
        json!({ "input": submit_input(None) }),
        &requester,
    )
    .await;
    assert!(
        probing.get("errors").is_some(),
        "the pre-flight payload must not carry a vote table: {probing}"
    );

    // A user with no request permission on the library is refused before any
    // rule runs.
    let stranger = user_with(
        &ctx,
        "no-request-permission",
        AppPermissionMask::NONE,
        LibraryPermissionMask::from_permissions([LibraryPermission::View]),
    )
    .await;
    let refused = gql_as(
        &ctx,
        "query($input: SubmitMediaRequestInput!) {
            previewMyRequestDecision(input: $input) { outcome }
        }",
        json!({ "input": submit_input(None) }),
        &stranger,
    )
    .await;
    assert!(
        refused.get("errors").is_some(),
        "a requester who may not ask may not use the rules as an oracle: {refused}"
    );
}

// ===========================================================================
// 6. The instance gate
// ===========================================================================

#[tokio::test]
async fn the_instance_gate_needs_system_settings_authority() {
    let ctx = TestContext::new().await;
    seed_request_rule_gate_definition(&ctx).await;

    let catalog_only = user_with(
        &ctx,
        "catalog-admin",
        AppPermissionMask::from_permissions([AppPermission::ManageCatalogSettings]),
        LibraryPermissionMask::NONE,
    )
    .await;

    let refused = gql_as(
        &ctx,
        "mutation($input: SetRequestRuleInstanceGatesInput!) {
            setRequestRuleInstanceGates(input: $input) { evaluationEnabled }
        }",
        json!({ "input": { "evaluationEnabled": true } }),
        &catalog_only,
    )
    .await;
    assert!(
        refused.get("errors").is_some(),
        "arming the instance is a system setting, not catalog administration: {refused}"
    );

    let read_refused = gql_as(
        &ctx,
        "query { requestRuleInstanceGates { evaluationEnabled } }",
        json!({}),
        &catalog_only,
    )
    .await;
    assert!(read_refused.get("errors").is_some());

    // Off by default.
    let gates = gql(
        &ctx,
        "query { requestRuleInstanceGates { evaluationEnabled } }",
        json!({}),
    )
    .await;
    assert_no_errors(&gates);
    assert_eq!(
        gates["data"]["requestRuleInstanceGates"]["evaluationEnabled"],
        false
    );

    arm_gate(&ctx, true).await;
    let armed = gql(
        &ctx,
        "query { requestRuleInstanceGates { evaluationEnabled } }",
        json!({}),
    )
    .await;
    assert_no_errors(&armed);
    assert_eq!(
        armed["data"]["requestRuleInstanceGates"]["evaluationEnabled"],
        true
    );
}

// ===========================================================================
// 7. Enforcement reaches the request
// ===========================================================================

#[tokio::test]
async fn an_enforced_approval_leases_the_title_and_explains_itself() {
    let ctx = TestContext::new().await;
    let requester = requester(&ctx, "approved-requester").await;

    let request = submit_under(&ctx, &requester, APPROVE_EVERYTHING, "ENFORCE", Some(30)).await;

    assert_eq!(request["status"], "APPROVED");
    assert_eq!(request["requestedLeaseDays"], 30);
    assert_eq!(request["policyTags"], json!(["auto-approved"]));
    assert!(
        request["createdTitleId"].as_str().is_some(),
        "an approval creates the title: {request}"
    );

    let lease = &request["lease"];
    assert_eq!(lease["requestedDays"], 30);
    // Dormant until the title actually imports: the clock starts at first
    // import, not at approval.
    assert_eq!(lease["state"], "DORMANT");
    assert!(lease["startsAt"].is_null() && lease["expiresAt"].is_null());

    // The metadata the request was decided against, read back out of the
    // snapshot captured at submit time.
    let metadata = &request["metadata"];
    assert_eq!(metadata["partial"], false, "metadata: {metadata}");
    assert_eq!(metadata["certificationLabel"], "PG-13");
    assert!(
        metadata["certificationRank"].as_i64().is_some(),
        "a US label on Scryer's ladder ranks: {metadata}"
    );
    assert_eq!(metadata["ageRating"], 13);
    assert_eq!(metadata["contentRatings"][0]["country"], "usa");
    assert_eq!(metadata["isAdult"], false);
    assert!(
        metadata["genres"]
            .as_array()
            .is_some_and(|genres| genres.iter().any(|genre| genre == "Action")),
        "the captured genres reach the approver: {metadata}"
    );

    let decision = &request["decision"];
    assert_eq!(decision["effectiveOutcome"], "AUTO_APPROVE");
    assert_eq!(decision["policyOutcome"], "AUTO_APPROVE");
    assert_eq!(decision["mode"], "ENFORCE");
    assert_eq!(decision["tags"], json!(["auto-approved"]));
    // The requester sees the verdict, never the vote table.
    assert!(
        decision["votes"].as_array().expect("votes").is_empty(),
        "a requester must not see the per-rule votes: {decision}"
    );

    // The claim the lease is derived from is visible to a library manager.
    let title_id = request["createdTitleId"].as_str().expect("title id");
    let claims = gql(
        &ctx,
        &format!("query($titleId: ID!) {{ titleClaims(titleId: $titleId) {{{CLAIM_FIELDS}}} }}"),
        json!({ "titleId": title_id }),
    )
    .await;
    assert_no_errors(&claims);
    let rows = claims["data"]["titleClaims"].as_array().expect("claims");
    assert_eq!(rows.len(), 1, "one approval, one claim: {claims}");
    assert_eq!(rows[0]["producer"], "REQUEST_LEASE");
    assert_eq!(rows[0]["kind"], "RETAIN_UNTIL");
    assert_eq!(rows[0]["state"], "DORMANT");
    assert_eq!(rows[0]["durationDays"], 30);
    assert_eq!(rows[0]["producerRef"], request["id"]);

    // A manager reading the same decision sees the whole trace.
    let trace = gql(
        &ctx,
        &format!("query($requestId: ID!) {{ requestRuleDecision(requestId: $requestId) {{{DECISION_FIELDS}}} }}"),
        json!({ "requestId": request["id"] }),
    )
    .await;
    assert_no_errors(&trace);
    let managed = &trace["data"]["requestRuleDecision"];
    assert_eq!(managed["effectiveOutcome"], "AUTO_APPROVE");
    assert_eq!(managed["votes"][0]["vote"], "APPROVE");
    assert_eq!(managed["votes"][0]["ruleSetName"], "Policy under test");
}

#[tokio::test]
async fn an_enforced_denial_rejects_the_request_and_shows_the_requester_why() {
    let ctx = TestContext::new().await;
    let requester = requester(&ctx, "denied-requester").await;

    let request = submit_under(&ctx, &requester, DENY_EVERYTHING, "ENFORCE", None).await;

    assert_eq!(request["status"], "REJECTED");
    assert!(
        request["createdTitleId"].is_null(),
        "a denial creates nothing: {request}"
    );
    assert!(request["lease"].is_null(), "a denial holds nothing");

    let decision = &request["decision"];
    assert_eq!(decision["policyOutcome"], "DENY");
    assert_eq!(decision["effectiveOutcome"], "DENY");
    assert!(decision["votes"].as_array().expect("votes").is_empty());
    assert_eq!(decision["reasons"][0]["code"], json!("policy_denied"));
    assert_eq!(
        decision["reasons"][0]["ruleName"],
        json!("Policy under test")
    );
}

#[tokio::test]
async fn a_shadow_rule_records_its_verdict_without_acting_on_it() {
    let ctx = TestContext::new().await;
    let requester = requester(&ctx, "shadow-requester").await;

    let request = submit_under(&ctx, &requester, DENY_EVERYTHING, "SHADOW", None).await;

    assert_eq!(
        request["status"], "PENDING",
        "shadow decides nothing: {request}"
    );
    let decision = &request["decision"];
    assert_eq!(decision["mode"], "SHADOW");
    assert_eq!(decision["policyOutcome"], "DENY");
    assert_ne!(
        decision["effectiveOutcome"], "DENY",
        "the effective outcome stays what the permission alone would have produced"
    );
}

// ===========================================================================
// 8. Administrator claim operations
// ===========================================================================

#[tokio::test]
async fn administrators_extend_convert_and_release_a_claim_and_requesters_cannot() {
    let ctx = TestContext::new().await;
    let requester = requester(&ctx, "claim-requester").await;

    let request = submit_under(&ctx, &requester, APPROVE_EVERYTHING, "ENFORCE", Some(30)).await;
    let title_id = request["createdTitleId"]
        .as_str()
        .expect("title id")
        .to_string();

    let claims = gql(
        &ctx,
        &format!("query($titleId: ID!) {{ titleClaims(titleId: $titleId) {{{CLAIM_FIELDS}}} }}"),
        json!({ "titleId": title_id }),
    )
    .await;
    assert_no_errors(&claims);
    let claim_id = claims["data"]["titleClaims"][0]["id"]
        .as_str()
        .expect("claim id")
        .to_string();

    // A plain requester may not read the claims, let alone change them.
    let refused_list = gql_as(
        &ctx,
        &format!("query($titleId: ID!) {{ titleClaims(titleId: $titleId) {{{CLAIM_FIELDS}}} }}"),
        json!({ "titleId": title_id }),
        &requester,
    )
    .await;
    assert!(refused_list.get("errors").is_some());

    let refused_release = gql_as(
        &ctx,
        &format!(
            "mutation($input: ReleaseTitleClaimInput!) {{ releaseTitleClaim(input: $input) {{{CLAIM_FIELDS}}} }}"
        ),
        json!({ "input": { "claimId": claim_id, "reason": "no longer wanted" } }),
        &requester,
    )
    .await;
    assert!(refused_release.get("errors").is_some());

    let expires_at = (chrono::Utc::now() + chrono::Duration::days(120)).to_rfc3339();
    let extended = gql(
        &ctx,
        &format!(
            "mutation($input: ExtendTitleClaimInput!) {{ extendTitleClaim(input: $input) {{{CLAIM_FIELDS}}} }}"
        ),
        json!({ "input": { "claimId": claim_id, "expiresAt": expires_at } }),
    )
    .await;
    assert_no_errors(&extended);
    assert!(
        extended["data"]["extendTitleClaim"]["expiresAt"]
            .as_str()
            .is_some(),
        "the extended window is stored: {extended}"
    );

    let converted = gql(
        &ctx,
        &format!(
            "mutation($input: ConvertTitleClaimToPermanentInput!) {{ convertTitleClaimToPermanent(input: $input) {{{CLAIM_FIELDS}}} }}"
        ),
        json!({ "input": { "claimId": claim_id } }),
    )
    .await;
    assert_no_errors(&converted);
    let replacement = &converted["data"]["convertTitleClaimToPermanent"];
    assert_eq!(replacement["producer"], "OPERATOR_KEEP");
    assert_eq!(replacement["kind"], "KEEP");
    assert_eq!(replacement["state"], "ACTIVE");
    assert!(
        replacement["producerRef"].is_null(),
        "an operator pin has nothing upstream to release against"
    );
    let replacement_id = replacement["id"].as_str().expect("replacement id");

    // The original stays as history rather than disappearing.
    let after_convert = gql(
        &ctx,
        &format!("query($titleId: ID!) {{ titleClaims(titleId: $titleId) {{{CLAIM_FIELDS}}} }}"),
        json!({ "titleId": title_id }),
    )
    .await;
    assert_no_errors(&after_convert);
    let original = after_convert["data"]["titleClaims"]
        .as_array()
        .expect("claims")
        .iter()
        .find(|claim| claim["id"] == json!(claim_id))
        .expect("the original claim is kept as history");
    assert_eq!(original["state"], "CONVERTED");

    let released = gql(
        &ctx,
        &format!(
            "mutation($input: ReleaseTitleClaimInput!) {{ releaseTitleClaim(input: $input) {{{CLAIM_FIELDS}}} }}"
        ),
        json!({ "input": { "claimId": replacement_id, "reason": "operator withdrew the pin" } }),
    )
    .await;
    assert_no_errors(&released);
    assert_eq!(released["data"]["releaseTitleClaim"]["state"], "RELEASED");
    assert_eq!(
        released["data"]["releaseTitleClaim"]["releasedReason"],
        "operator withdrew the pin"
    );
}

// ===========================================================================
// 9. The decision browser
// ===========================================================================

#[tokio::test]
async fn recent_decisions_filter_by_outcome_and_need_catalog_authority() {
    let ctx = TestContext::new().await;
    let requester = requester(&ctx, "browsed-requester").await;

    submit_under(&ctx, &requester, DENY_EVERYTHING, "ENFORCE", None).await;

    let denials = gql(
        &ctx,
        &format!(
            "query($outcome: RequestDecisionOutcomeValue) {{
                requestRuleDecisions(limit: 20, outcome: $outcome) {{{DECISION_FIELDS}}}
            }}"
        ),
        json!({ "outcome": "DENY" }),
    )
    .await;
    assert_no_errors(&denials);
    let rows = denials["data"]["requestRuleDecisions"]
        .as_array()
        .expect("decisions");
    assert!(
        !rows.is_empty(),
        "the denied request should be browsable: {denials}"
    );
    for row in rows {
        assert_eq!(row["effectiveOutcome"], "DENY");
    }
    // A catalog administrator is the audience the vote table exists for.
    assert!(
        rows.iter()
            .any(|row| !row["votes"].as_array().expect("votes").is_empty()),
        "an administrator browsing decisions sees the votes: {denials}"
    );

    let approvals = gql(
        &ctx,
        &format!(
            "query($outcome: RequestDecisionOutcomeValue) {{
                requestRuleDecisions(limit: 20, outcome: $outcome) {{{DECISION_FIELDS}}}
            }}"
        ),
        json!({ "outcome": "AUTO_APPROVE" }),
    )
    .await;
    assert_no_errors(&approvals);
    assert!(
        approvals["data"]["requestRuleDecisions"]
            .as_array()
            .expect("decisions")
            .is_empty(),
        "nothing was approved: {approvals}"
    );

    let refused = gql_as(
        &ctx,
        &format!("query {{ requestRuleDecisions(limit: 5) {{{DECISION_FIELDS}}} }}"),
        json!({}),
        &requester,
    )
    .await;
    assert!(
        refused.get("errors").is_some(),
        "browsing every decision is catalog administration: {refused}"
    );

    // The reference the authoring UI renders is served from the crate's own copy.
    let reference = gql(&ctx, "query { requestRuleInputReference }", json!({})).await;
    assert_no_errors(&reference);
    assert!(
        reference["data"]["requestRuleInputReference"]["sections"].is_array(),
        "the contract should expose its sections: {reference}"
    );
}
