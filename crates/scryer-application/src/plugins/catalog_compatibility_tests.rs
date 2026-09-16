use super::*;
use serde_json::{Value, json};

fn catalog_fixture() -> Value {
    json!({
        "schema_version": CATALOG_V3_SCHEMA_VERSION,
        "catalog_version": 58,
        "plugins": [{
            "id": "example", "name": "Example", "description": "Fixture",
            "plugin_type": "indexer", "provider_type": "example",
            "publisher": "example", "support_tier": "official", "status": "active",
            "docs_url": "https://github.com/example/plugin",
            "source_repo": "https://github.com/example/plugin",
            "required_signer": {"github_repository": "example/plugin"},
            "releases": [{
                "version": "1.0.0", "sdk_constraint": "^3.0",
                "artifacts": [{
                    "runtime": "wasm32-wasip2", "required_features": [],
                    "url": "https://example.test/plugin.wasm.zst",
                    "signature_url": "https://example.test/plugin.bundle.zst",
                    "digests": ["blake3:11"], "wasm_digests": ["blake3:22"], "bytes": 1234
                }]
            }]
        }],
        "community_sources": [{
            "id": "community", "github_repository": "example/community",
            "support_tier": "verified_community"
        }],
        "rule_packs": [{
            "id": "example-pack", "name": "Example pack", "description": "Fixture",
            "author": "example",
            "releases": [{
                "version": "1.0.0", "min_scryer_version": "0.20.0",
                "customizable": false,
                "rule_pack_digests": ["blake3:33"], "rule_pack_bytes": 123,
                "artifacts": [{
                    "url": "https://example.test/pack.json.zst",
                    "signature_url": "https://example.test/pack.bundle.zst",
                    "digests": ["blake3:44"]
                }]
            }]
        }]
    })
}

fn redirect_fixture() -> Value {
    json!({
        "schema_version": CATALOG_V3_REDIRECT_SCHEMA_VERSION,
        "catalog_version": 58,
        "artifacts": [{
            "url": "https://example.test/catalog.json.zst",
            "signature_url": "https://example.test/catalog.bundle.zst"
        }]
    })
}

// Exercise every wire object, including signer, community source, rule-pack
// release and distribution artifact, rather than only the catalog envelope.
fn add_future_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for child in object.values_mut() {
                add_future_fields(child);
            }
            object.insert("future_boolean".into(), json!(false));
            object.insert(
                "future_metadata".into(),
                json!({"values": [1, null, "new"]}),
            );
        }
        Value::Array(array) => array.iter_mut().for_each(add_future_fields),
        _ => {}
    }
}

#[test]
fn catalog_additive_fields_at_every_level_preserve_known_values() {
    let mut fixture = catalog_fixture();
    let baseline = parse_and_validate_catalog_v3(&serde_json::to_vec(&fixture).unwrap()).unwrap();
    add_future_fields(&mut fixture);
    let extended = parse_and_validate_catalog_v3(&serde_json::to_vec(&fixture).unwrap())
        .expect("additive fields must not break catalog refresh");
    assert_eq!(
        serde_json::to_value(extended).unwrap(),
        serde_json::to_value(baseline).unwrap()
    );
}

#[test]
fn catalog_additive_fields_do_not_require_rule_pack_customizable() {
    let mut fixture = catalog_fixture();
    fixture["rule_packs"][0]["releases"][0]
        .as_object_mut()
        .unwrap()
        .remove("customizable");
    add_future_fields(&mut fixture);
    let catalog = parse_and_validate_catalog_v3(&serde_json::to_vec(&fixture).unwrap()).unwrap();
    assert!(catalog.rule_packs[0].releases[0].customizable);
}

#[test]
fn catalog_additive_fields_in_redirect_preserve_known_values() {
    let mut fixture = redirect_fixture();
    let baseline =
        parse_and_validate_catalog_v3_redirect(&serde_json::to_vec(&fixture).unwrap()).unwrap();
    add_future_fields(&mut fixture);
    let extended = parse_and_validate_catalog_v3_redirect(&serde_json::to_vec(&fixture).unwrap())
        .expect("additive fields must not break redirect resolution");
    assert_eq!(
        serde_json::to_value(extended).unwrap(),
        serde_json::to_value(baseline).unwrap()
    );
}

#[test]
fn catalog_additive_fields_do_not_relax_known_field_validation() {
    for (pointer, invalid) in [
        ("/schema_version", json!("scryer.plugin.catalog.v99")),
        ("/catalog_version", json!(0)),
        (
            "/plugins/0/required_signer/github_repository",
            json!("another/publisher"),
        ),
        (
            "/plugins/0/releases/0/artifacts/0/bytes",
            json!("not a number"),
        ),
        ("/rule_packs/0/releases/0/customizable", json!("false")),
        (
            "/rule_packs/0/releases/0/min_scryer_version",
            json!("not a version"),
        ),
        ("/rule_packs/0/releases/0/rule_pack_digests", json!([])),
        (
            "/rule_packs/0/releases/0/artifacts/0/signature_url",
            json!(null),
        ),
    ] {
        let mut fixture = catalog_fixture();
        add_future_fields(&mut fixture);
        *fixture.pointer_mut(pointer).unwrap() = invalid;
        assert!(
            parse_and_validate_catalog_v3(&serde_json::to_vec(&fixture).unwrap()).is_err(),
            "accepted invalid known field {pointer}"
        );
    }

    let mut fixture = catalog_fixture();
    add_future_fields(&mut fixture);
    fixture["plugins"][0]["required_signer"]
        .as_object_mut()
        .unwrap()
        .remove("github_repository");
    assert!(
        parse_and_validate_catalog_v3(&serde_json::to_vec(&fixture).unwrap()).is_err(),
        "a missing signer must still fail"
    );
}

#[test]
fn catalog_additive_fields_do_not_relax_redirect_validation() {
    for (pointer, invalid) in [
        (
            "/schema_version",
            json!("scryer.plugin.catalog.v99.redirect"),
        ),
        ("/catalog_version", json!(0)),
        ("/artifacts", json!([])),
        (
            "/artifacts/0/signature_url",
            json!("file:///fixture/signature"),
        ),
    ] {
        let mut fixture = redirect_fixture();
        add_future_fields(&mut fixture);
        *fixture.pointer_mut(pointer).unwrap() = invalid;
        assert!(
            parse_and_validate_catalog_v3_redirect(&serde_json::to_vec(&fixture).unwrap()).is_err(),
            "accepted invalid redirect field {pointer}"
        );
    }
}
