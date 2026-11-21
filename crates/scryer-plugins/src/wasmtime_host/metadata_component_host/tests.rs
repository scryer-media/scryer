//! Host dispatch tests for the metadata-provider world.
//!
//! The fixture components are the subtitle host's WAT, as the list host's
//! tests use: the metadata world imports the same shared door and runtime and
//! exports the same `describe` / `process` pair. These tests prove that the
//! metadata world binds against the list host's shared import implementations,
//! describes itself, and round-trips the metadata command envelope.

use scryer_plugin_sdk::command::{
    PluginCommand, PluginCommandRequest, PluginCommandResponse, PluginCommandResult,
    PluginMetadataCommand, PluginMetadataCommandResult,
};
use scryer_plugin_sdk::{
    MetadataMediaKind, MetadataPluginLookupRequest, MetadataPluginLookupResponse,
    MetadataPluginTitle, MetadataProviderDescriptor, PluginDescriptor, PluginError,
    PluginErrorCode, PluginResult, ProviderDescriptor,
};

use super::*;
use crate::wasmtime_host::subtitle_component_host::tests as fixtures;

const FIXTURE_IMDB_ID: &str = "tt90000001";

fn metadata_descriptor_json(id: &str) -> String {
    let provider: MetadataProviderDescriptor = serde_json::from_value(serde_json::json!({
        "provider_type": "fixture-metadata",
        "display_name": "Fixture Metadata",
    }))
    .expect("minimal metadata descriptor");
    serde_json::to_string(&PluginDescriptor {
        settings: Vec::new(),
        id: id.to_string(),
        name: "Fixture Metadata".to_string(),
        version: "1.0.0".to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: Vec::new(),
        provider: ProviderDescriptor::MetadataProvider(provider),
    })
    .expect("fixture descriptor must serialize")
}

fn lookup_response_json(title: &str) -> String {
    serde_json::to_string(&PluginCommandResponse::new(PluginCommandResult::Metadata(
        PluginMetadataCommandResult::Lookup(PluginResult::Ok(MetadataPluginLookupResponse {
            title: Some(MetadataPluginTitle {
                imdb_id: FIXTURE_IMDB_ID.to_string(),
                title: title.to_string(),
                year: Some(2001),
                ..MetadataPluginTitle::default()
            }),
        })),
    )))
    .expect("fixture response must serialize")
}

fn lookup_failure_json() -> String {
    serde_json::to_string(&PluginCommandResponse::new(PluginCommandResult::Metadata(
        PluginMetadataCommandResult::Lookup(PluginResult::Err(PluginError {
            code: PluginErrorCode::Permanent,
            public_message: "metadata host binding mismatch".to_string(),
            debug_message: None,
            retry_after_seconds: None,
            details: None,
        })),
    )))
    .expect("fixture failure must serialize")
}

fn host_call_fixture() -> Vec<u8> {
    wat::parse_str(fixtures::fixture_component_wat(
        &metadata_descriptor_json("fixture-metadata"),
        &lookup_response_json("Host Call Verified"),
        &lookup_failure_json(),
        &fixtures::host_request_bytes(),
        &fixtures::expected_host_response_bytes(),
    ))
    .expect("fixture metadata component WAT must assemble")
}

fn runtime_fixture() -> Vec<u8> {
    wat::parse_str(fixtures::fixture_component_v1_1_wat(
        &metadata_descriptor_json("fixture-metadata-runtime"),
        &lookup_response_json("Runtime Verified"),
        &lookup_failure_json(),
        fixtures::V1_1_FORBIDDEN_ORIGIN,
    ))
    .expect("fixture metadata runtime component WAT must assemble")
}

fn lookup_request() -> PluginCommandRequest {
    PluginCommandRequest::new(PluginCommand::Metadata(PluginMetadataCommand::Lookup(
        MetadataPluginLookupRequest {
            kind: MetadataMediaKind::Movie,
            imdb_id: FIXTURE_IMDB_ID.to_string(),
        },
    )))
}

fn invocation() -> MetadataComponentInvocation<'static> {
    MetadataComponentInvocation {
        plugin_id: "fixture-metadata",
        plugin_version: "1.0.0",
        operation: "lookup",
    }
}

fn lookup_result(response: PluginCommandResponse) -> PluginResult<MetadataPluginLookupResponse> {
    let PluginCommandResult::Metadata(PluginMetadataCommandResult::Lookup(result)) =
        response.response
    else {
        panic!("fixture must answer a metadata lookup with a metadata lookup result");
    };
    result
}

#[test]
fn an_arbitrary_component_fails_metadata_world_validation() {
    let wasm = wat::parse_str("(component)").expect("component WAT must parse");
    let error = validate_metadata_component(&wasm)
        .expect_err("an arbitrary component must not pass metadata-world validation");
    assert!(
        error.contains("scryer:metadata/metadata-provider@1.0.0"),
        "{error}"
    );
}

#[test]
fn both_fixtures_pass_metadata_world_validation() {
    validate_metadata_component(&host_call_fixture()).expect("host-call fixture must bind");
    validate_metadata_component(&runtime_fixture()).expect("runtime fixture must bind");
}

#[test]
fn describe_returns_a_metadata_provider_descriptor() {
    let descriptor =
        metadata_component_describe(&host_call_fixture()).expect("fixture must self-describe");
    assert_eq!(descriptor.id, "fixture-metadata");
    assert_eq!(
        descriptor.kind(),
        scryer_plugin_sdk::PluginKind::MetadataProvider
    );
    assert_eq!(descriptor.provider_type(), "fixture-metadata");
}

#[tokio::test(flavor = "multi_thread")]
async fn process_round_trips_a_lookup_through_the_shared_host_call() {
    let spec = fixtures::test_spec(host_call_fixture(), fixtures::configured_command_host());
    let response = process_metadata_component(&spec, &lookup_request(), invocation())
        .await
        .expect("the fixture must complete one process exchange");
    let PluginResult::Ok(lookup) = lookup_result(response) else {
        panic!("a plugin error means the shared host-call did not round-trip intact");
    };
    let title = lookup.title.expect("the fixture answers with a title");
    assert_eq!(title.imdb_id, FIXTURE_IMDB_ID);
    assert_eq!(title.title, "Host Call Verified");
}

#[tokio::test(flavor = "multi_thread")]
async fn process_drives_the_async_export_and_the_shared_typed_runtime() {
    let spec = fixtures::test_spec(runtime_fixture(), fixtures::configured_command_host());
    let response = process_metadata_component(&spec, &lookup_request(), invocation())
        .await
        .expect("the runtime fixture must complete one process exchange");
    let PluginResult::Ok(lookup) = lookup_result(response) else {
        panic!("a plugin error means a typed runtime import did not behave as promised");
    };
    assert_eq!(
        lookup
            .title
            .expect("the fixture answers with a title")
            .title,
        "Runtime Verified"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_host_answers_in_band_rather_than_failing_the_call() {
    let spec = fixtures::test_spec(host_call_fixture(), CommandHost::disabled());
    let response = process_metadata_component(&spec, &lookup_request(), invocation())
        .await
        .expect("a disabled host must not fail the invocation itself");
    assert!(matches!(lookup_result(response), PluginResult::Err(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_json_response_is_a_protocol_failure() {
    let wasm = wat::parse_str(fixtures::fixture_component_wat(
        &metadata_descriptor_json("fixture-metadata"),
        "not json at all",
        "not json either",
        &fixtures::host_request_bytes(),
        &fixtures::expected_host_response_bytes(),
    ))
    .expect("fixture metadata component WAT must assemble");
    let spec = fixtures::test_spec(wasm, fixtures::configured_command_host());
    let error = process_metadata_component(&spec, &lookup_request(), invocation())
        .await
        .expect_err("a malformed response document must fail the invocation");
    assert!(
        error.to_string().contains("PluginCommandResponse"),
        "{error}"
    );
}
