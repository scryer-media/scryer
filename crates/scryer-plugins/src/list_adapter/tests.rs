//! Provider and client tests for list plugins, driven by the hand-built
//! runtime fixture component.

use std::collections::BTreeMap;

use scryer_application::{ListPluginProvider, RuntimePluginLoad};
use scryer_plugin_sdk::{
    ListCredential, ListPluginFetchRequest, PluginDescriptor, PluginResult, ProviderDescriptor,
};

use super::*;
use crate::wasmtime_host::list_component_host::tests as list_fixtures;
use crate::wasmtime_host::subtitle_component_host::tests as fixtures;

fn descriptor(provider: serde_json::Value) -> PluginDescriptor {
    PluginDescriptor {
        id: "fixture-lists".to_string(),
        name: "Fixture Lists".to_string(),
        version: "1.0.0".to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: Vec::new(),
        provider: ProviderDescriptor::ListProvider(
            serde_json::from_value(provider).expect("list descriptor"),
        ),
    }
}

fn runtime_plugin(descriptor: PluginDescriptor) -> RuntimePluginLoad {
    let wasm = wat::parse_str(fixtures::fixture_component_v1_1_wat(
        &serde_json::to_string(&descriptor).expect("descriptor json"),
        &list_fixtures::fetch_response_json("runtime-verified"),
        &list_fixtures::fetch_failure_json(),
        fixtures::V1_1_FORBIDDEN_ORIGIN,
    ))
    .expect("fixture list component WAT must assemble");
    RuntimePluginLoad {
        descriptor,
        wasm_bytes: wasm,
        first_party: false,
    }
}

fn fetch_request(credential: Option<ListCredential>) -> ListPluginFetchRequest {
    ListPluginFetchRequest {
        source_type: "user_list".to_string(),
        params: BTreeMap::from([("list_id".to_string(), "fixture-list".to_string())]),
        credential,
        page_cursor: None,
        since_fingerprint: None,
    }
}

fn member_credential() -> ListCredential {
    ListCredential {
        access_token: "fixture-token".to_string(),
        token_type: None,
        external_user_id: None,
        username: None,
    }
}

#[test]
fn a_runtime_plugin_is_found_by_type_and_alias() {
    let provider = DynamicListPluginProvider::new(WasmListPluginProvider::empty());
    provider
        .upsert_runtime_plugin(runtime_plugin(descriptor(serde_json::json!({
            "provider_type": "fixture-lists",
            "provider_aliases": ["fixture-alias"],
        }))))
        .expect("upsert");

    assert_eq!(provider.available_provider_types(), vec!["fixture-lists"]);
    assert_eq!(provider.descriptors().len(), 1);
    assert!(
        provider
            .client_for_provider("FIXTURE-ALIAS", &BTreeMap::new())
            .is_some()
    );

    provider
        .remove_runtime_plugin("fixture-lists")
        .expect("remove");
    assert!(provider.available_provider_types().is_empty());
    assert!(
        provider
            .client_for_provider("fixture-lists", &BTreeMap::new())
            .is_none()
    );
}

#[test]
fn a_descriptor_of_another_family_is_refused() {
    let mut subtitle = descriptor(serde_json::json!({ "provider_type": "fixture-lists" }));
    subtitle.provider = ProviderDescriptor::Subtitle(
        serde_json::from_value(serde_json::json!({ "provider_type": "fixture-subtitles" }))
            .expect("subtitle descriptor"),
    );
    let provider = WasmListPluginProvider::empty().with_runtime_plugin(RuntimePluginLoad {
        descriptor: subtitle,
        wasm_bytes: Vec::new(),
        first_party: false,
    });
    assert!(provider.available_provider_types().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_round_trips_through_the_client() {
    let provider = WasmListPluginProvider::empty().with_runtime_plugin(runtime_plugin(descriptor(
        serde_json::json!({ "provider_type": "fixture-lists" }),
    )));
    let client = provider
        .client_for_provider("fixture-lists", &BTreeMap::new())
        .expect("client");
    let PluginResult::Ok(page) = client.fetch(fetch_request(None)).await.expect("fetch") else {
        panic!("the fixture answers a fetch with items");
    };
    assert_eq!(page.items[0].item_key, "runtime-verified");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_only_provider_is_never_invoked_without_a_credential() {
    let provider = WasmListPluginProvider::empty().with_runtime_plugin(runtime_plugin(descriptor(
        serde_json::json!({
            "provider_type": "fixture-lists",
            "capabilities": { "requires_member_credential": true },
        }),
    )));
    let client = provider
        .client_for_provider("fixture-lists", &BTreeMap::new())
        .expect("client");

    let error = client
        .fetch(fetch_request(None))
        .await
        .expect_err("a credential-less fetch must be refused");
    assert!(matches!(error, AppError::Validation(_)), "{error}");

    let PluginResult::Ok(page) = client
        .fetch(fetch_request(Some(member_credential())))
        .await
        .expect("fetch with a credential")
    else {
        panic!("the fixture answers a fetch with items");
    };
    assert_eq!(page.items.len(), 1);
}

fn feed_descriptor() -> PluginDescriptor {
    descriptor(serde_json::json!({
        "provider_type": "fixture-feeds",
        "allowed_hosts": ["api.fixture-feeds.test"],
        "rate_limit_seconds": 30,
        "groups": [{
            "label": "Feeds",
            "auth_badge": "no_account_needs_value",
            "items": [{
                "id": "custom",
                "name": "Custom feed",
                "source_type": "custom_feed",
                "default_interval_seconds": 3600,
                "params": [
                    { "key": "feed_url", "label": "Feed link", "type": "url", "required": true },
                    { "key": "label", "label": "Label", "type": "text" }
                ]
            }]
        }]
    }))
}

fn feed_request(source_type: &str, feed_url: &str) -> ListPluginFetchRequest {
    ListPluginFetchRequest {
        source_type: source_type.to_string(),
        params: BTreeMap::from([
            ("feed_url".to_string(), feed_url.to_string()),
            (
                "label".to_string(),
                "https://text-param.fixture.test/not-a-link".to_string(),
            ),
        ]),
        credential: None,
        page_cursor: None,
        since_fingerprint: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_feed_host_is_allowed_only_for_the_call_that_links_it() {
    let provider =
        WasmListPluginProvider::empty().with_runtime_plugin(runtime_plugin(feed_descriptor()));
    let loaded = resolve_loaded_plugin(&provider.plugins, &provider.aliases, "fixture-feeds")
        .expect("loaded plugin");
    let client = WasmListPluginProvider::build_client(loaded, &BTreeMap::new(), &provider.pacer)
        .expect("client");

    let linked = client.allowed_hosts_for_fetch(&feed_request(
        "custom_feed",
        "https://Feeds.Example-Fixture.test/list.json",
    ));
    assert_eq!(
        linked,
        vec![
            "api.fixture-feeds.test".to_string(),
            "feeds.example-fixture.test".to_string()
        ],
        "the linked host joins the declared ones; a text parameter never does"
    );

    // Another call on the same client, without that link, gets the declared
    // hosts only.
    let other = client.allowed_hosts_for_fetch(&feed_request(
        "user_list",
        "https://feeds.example-fixture.test/list.json",
    ));
    assert_eq!(other, vec!["api.fixture-feeds.test".to_string()]);

    // A link that is not http(s) adds nothing.
    let ftp = client.allowed_hosts_for_fetch(&feed_request(
        "custom_feed",
        "ftp://feeds.example-fixture.test/list.json",
    ));
    assert_eq!(ftp, vec!["api.fixture-feeds.test".to_string()]);
}

#[test]
fn the_fetch_interval_comes_from_the_descriptor() {
    assert_eq!(fetch_interval(&feed_descriptor()), Duration::from_secs(30));
    assert_eq!(
        fetch_interval(&descriptor(
            serde_json::json!({ "provider_type": "fixture-lists" })
        )),
        Duration::ZERO
    );
    assert_eq!(
        fetch_interval(&descriptor(serde_json::json!({
            "provider_type": "fixture-lists",
            "rate_limit_seconds": -5,
        }))),
        Duration::ZERO
    );
}

#[test]
fn fetch_calls_to_one_provider_are_spaced_by_its_interval() {
    let pacer = ListFetchPacer::default();
    let interval = Duration::from_secs(30);
    let start = tokio::time::Instant::now();

    assert_eq!(
        pacer.reserve("fixture-feeds", interval, start),
        Some(start),
        "the first call never waits"
    );
    assert_eq!(
        pacer.reserve("fixture-other", interval, start),
        Some(start),
        "another provider has its own slot"
    );
    assert_eq!(pacer.reserve("fixture-feeds", Duration::ZERO, start), None);

    assert_eq!(
        pacer.reserve("FIXTURE-FEEDS", interval, start),
        Some(start + interval),
        "the second call waits out the interval"
    );
    assert_eq!(
        pacer.reserve("fixture-feeds", interval, start + Duration::from_secs(5)),
        Some(start + interval * 2)
    );
    // A call long after the last slot runs at once.
    let later = start + Duration::from_secs(600);
    assert_eq!(pacer.reserve("fixture-feeds", interval, later), Some(later));
}

#[test]
fn a_reload_keeps_the_fetch_pacing() {
    let provider = DynamicListPluginProvider::new(WasmListPluginProvider::empty());
    let before = provider.inner.read().unwrap().pacer.next_call.clone();
    provider.reload(WasmListPluginProvider::empty());
    let after = provider.inner.read().unwrap().pacer.next_call.clone();
    assert!(Arc::ptr_eq(&before, &after));
}
