#![recursion_limit = "256"]

mod common;

use chrono::Utc;
use serde_json::json;

use common::TestContext;
use scryer_domain::{
    DomainEventPayload, DomainEventStream, Id, LibraryScanProgressedEventData,
    LibraryScanStartedEventData, MediaFacet, NewDomainEvent,
};

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

    ctx.app
        .services
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: None,
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::LibraryScan {
                session_id: "session-1".to_string(),
            },
            payload: DomainEventPayload::LibraryScanStarted(LibraryScanStartedEventData {
                session_id: "session-1".to_string(),
                mode: "full".to_string(),
            }),
        })
        .await
        .expect("append library scan started event");
    ctx.app
        .services
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: None,
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::LibraryScan {
                session_id: "session-1".to_string(),
            },
            payload: DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: "session-1".to_string(),
                status: "running".to_string(),
                found_titles: 12,
                titles_completed: 2,
                titles_total: Some(4),
                files_completed: 5,
                files_total: Some(9),
            }),
        })
        .await
        .expect("append library scan progressed event");

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
    assert_eq!(scans[0]["sessionId"], "session-1");
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
