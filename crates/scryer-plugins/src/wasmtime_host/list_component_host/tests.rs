//! Host dispatch tests for the list-provider world.
//!
//! The hand-built fixture components are the subtitle host's: both families
//! import the same shared door and runtime packages and export the same
//! `describe` / `process` pair, so the WAT is reused and only the descriptor
//! and response documents differ. What these tests prove is that the list
//! world binds, describes, and round-trips the list command envelope through
//! the shared service layer.

use std::collections::BTreeMap;

use scryer_plugin_sdk::command::{
    PluginCommand, PluginCommandRequest, PluginCommandResponse, PluginCommandResult,
    PluginListCommand, PluginListCommandResult,
};
use scryer_plugin_sdk::{
    ListMediaKind, ListPluginFetchRequest, ListPluginFetchResponse, ListPluginItem,
    ListProviderDescriptor, PluginDescriptor, PluginError, PluginErrorCode, PluginResult,
    ProviderDescriptor,
};

use super::*;
use crate::wasmtime_host::subtitle_component_host::tests as fixtures;

fn list_descriptor_json(id: &str) -> String {
    let provider: ListProviderDescriptor =
        serde_json::from_value(serde_json::json!({ "provider_type": "fixture-lists" }))
            .expect("minimal list descriptor");
    serde_json::to_string(&PluginDescriptor {
        id: id.to_string(),
        name: "Fixture Lists".to_string(),
        version: "1.0.0".to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: Vec::new(),
        provider: ProviderDescriptor::ListProvider(provider),
    })
    .expect("fixture descriptor must serialize")
}

pub(crate) fn fetch_response_json(item_key: &str) -> String {
    serde_json::to_string(&PluginCommandResponse::new(PluginCommandResult::List(
        PluginListCommandResult::Fetch(PluginResult::Ok(ListPluginFetchResponse {
            items: vec![ListPluginItem {
                item_key: item_key.to_string(),
                rank: Some(1),
                kind_hint: Some(ListMediaKind::Movie),
                ..ListPluginItem::default()
            }],
            ..ListPluginFetchResponse::default()
        })),
    )))
    .expect("fixture response must serialize")
}

pub(crate) fn fetch_failure_json() -> String {
    serde_json::to_string(&PluginCommandResponse::new(PluginCommandResult::List(
        PluginListCommandResult::Fetch(PluginResult::Err(PluginError {
            code: PluginErrorCode::Permanent,
            public_message: "list host binding mismatch".to_string(),
            debug_message: None,
            retry_after_seconds: None,
            details: None,
        })),
    )))
    .expect("fixture failure must serialize")
}

fn host_call_fixture() -> Vec<u8> {
    wat::parse_str(fixtures::fixture_component_wat(
        &list_descriptor_json("fixture-lists"),
        &fetch_response_json("host-call-verified"),
        &fetch_failure_json(),
        &fixtures::host_request_bytes(),
        &fixtures::expected_host_response_bytes(),
    ))
    .expect("fixture list component WAT must assemble")
}

fn runtime_fixture() -> Vec<u8> {
    wat::parse_str(fixtures::fixture_component_v1_1_wat(
        &list_descriptor_json("fixture-lists-runtime"),
        &fetch_response_json("runtime-verified"),
        &fetch_failure_json(),
        fixtures::V1_1_FORBIDDEN_ORIGIN,
    ))
    .expect("fixture list runtime component WAT must assemble")
}

fn fetch_request() -> PluginCommandRequest {
    PluginCommandRequest::new(PluginCommand::List(PluginListCommand::Fetch(
        ListPluginFetchRequest {
            source_type: "user_list".to_string(),
            params: BTreeMap::from([("list_id".to_string(), "fixture-list".to_string())]),
            credential: None,
            page_cursor: None,
            since_fingerprint: None,
        },
    )))
}

fn invocation() -> ListComponentInvocation<'static> {
    ListComponentInvocation {
        plugin_id: "fixture-lists",
        plugin_version: "1.0.0",
        operation: "fetch",
    }
}

fn fetch_result(response: PluginCommandResponse) -> PluginResult<ListPluginFetchResponse> {
    let PluginCommandResult::List(PluginListCommandResult::Fetch(result)) = response.response
    else {
        panic!("fixture must answer a list fetch with a list fetch result");
    };
    result
}

#[test]
fn an_arbitrary_component_fails_list_world_validation() {
    let wasm = wat::parse_str("(component)").expect("component WAT must parse");
    let error = validate_list_component(&wasm)
        .expect_err("an arbitrary component must not pass list-world validation");
    assert!(
        error.contains("scryer:lists/list-provider@1.0.0"),
        "{error}"
    );
}

#[test]
fn both_fixtures_pass_list_world_validation() {
    validate_list_component(&host_call_fixture()).expect("host-call fixture must bind");
    validate_list_component(&runtime_fixture()).expect("runtime fixture must bind");
}

#[test]
fn describe_returns_a_list_provider_descriptor() {
    let descriptor =
        list_component_describe(&host_call_fixture()).expect("fixture must self-describe");
    assert_eq!(descriptor.id, "fixture-lists");
    assert_eq!(
        descriptor.kind(),
        scryer_plugin_sdk::PluginKind::ListProvider
    );
    assert_eq!(descriptor.provider_type(), "fixture-lists");
}

#[tokio::test(flavor = "multi_thread")]
async fn process_round_trips_a_fetch_through_the_shared_host_call() {
    let spec = fixtures::test_spec(host_call_fixture(), fixtures::configured_command_host());
    let response = process_list_component(&spec, &fetch_request(), invocation())
        .await
        .expect("the fixture must complete one process exchange");
    let PluginResult::Ok(fetch) = fetch_result(response) else {
        panic!("a plugin error means the shared host-call did not round-trip intact");
    };
    assert_eq!(fetch.items.len(), 1);
    assert_eq!(fetch.items[0].item_key, "host-call-verified");
}

#[tokio::test(flavor = "multi_thread")]
async fn process_drives_the_async_export_and_the_typed_runtime() {
    let spec = fixtures::test_spec(runtime_fixture(), fixtures::configured_command_host());
    let response = process_list_component(&spec, &fetch_request(), invocation())
        .await
        .expect("the runtime fixture must complete one process exchange");
    let PluginResult::Ok(fetch) = fetch_result(response) else {
        panic!("a plugin error means a typed runtime import did not behave as promised");
    };
    assert_eq!(fetch.items[0].item_key, "runtime-verified");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_host_answers_in_band_rather_than_failing_the_call() {
    let spec = fixtures::test_spec(host_call_fixture(), CommandHost::disabled());
    let response = process_list_component(&spec, &fetch_request(), invocation())
        .await
        .expect("a disabled host must not fail the invocation itself");
    assert!(matches!(fetch_result(response), PluginResult::Err(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_json_response_is_a_protocol_failure() {
    let wasm = wat::parse_str(fixtures::fixture_component_wat(
        &list_descriptor_json("fixture-lists"),
        "not json at all",
        "not json either",
        &fixtures::host_request_bytes(),
        &fixtures::expected_host_response_bytes(),
    ))
    .expect("fixture list component WAT must assemble");
    let spec = fixtures::test_spec(wasm, fixtures::configured_command_host());
    let error = process_list_component(&spec, &fetch_request(), invocation())
        .await
        .expect_err("a malformed response document must fail the invocation");
    assert!(
        error.to_string().contains("PluginCommandResponse"),
        "{error}"
    );
}
