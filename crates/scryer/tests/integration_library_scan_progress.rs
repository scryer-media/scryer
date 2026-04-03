#![recursion_limit = "256"]

mod common;

use serde_json::json;

use common::TestContext;
use scryer_domain::MediaFacet;

async fn gql(ctx: &TestContext, query: &str, variables: serde_json::Value) -> serde_json::Value {
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

fn assert_no_errors(body: &serde_json::Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

#[tokio::test]
async fn active_library_scans_query_returns_progress_snapshot() {
    let ctx = TestContext::new().await;

    let session = ctx
        .app
        .services
        .library_scan_tracker
        .start_session(MediaFacet::Series)
        .await
        .expect("start library scan session");
    ctx.app
        .services
        .library_scan_tracker
        .set_found_titles(&session.session_id, 12)
        .await;
    ctx.app
        .services
        .library_scan_tracker
        .add_metadata_total(&session.session_id, 4)
        .await;
    ctx.app
        .services
        .library_scan_tracker
        .increment_metadata_completed(&session.session_id, 2)
        .await;
    ctx.app
        .services
        .library_scan_tracker
        .add_file_total(&session.session_id, 9)
        .await;
    ctx.app
        .services
        .library_scan_tracker
        .increment_file_completed(&session.session_id, 5)
        .await;

    let body = gql(
        &ctx,
        r#"query { activeLibraryScans { sessionId facet status foundTitles metadataProgress { total completed failed } fileProgress { total completed failed } } }"#,
        json!({}),
    )
    .await;

    assert_no_errors(&body);
    let scans = body["data"]["activeLibraryScans"]
        .as_array()
        .expect("activeLibraryScans should be an array");
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0]["sessionId"], session.session_id);
    assert_eq!(scans[0]["facet"], "tv");
    assert_eq!(scans[0]["status"], "running");
    assert_eq!(scans[0]["foundTitles"], 12);
    assert_eq!(scans[0]["metadataProgress"]["total"], 4);
    assert_eq!(scans[0]["metadataProgress"]["completed"], 2);
    assert_eq!(scans[0]["metadataProgress"]["failed"], 0);
    assert_eq!(scans[0]["fileProgress"]["total"], 9);
    assert_eq!(scans[0]["fileProgress"]["completed"], 5);
    assert_eq!(scans[0]["fileProgress"]["failed"], 0);
}
