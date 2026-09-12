use super::*;

fn pack_fixture() -> CatalogV3RulePackEntry {
    let release = |version: &str, minimum: Option<&str>| CatalogV3RulePackRelease {
        version: version.into(),
        customizable: true,
        min_scryer_version: minimum.map(String::from),
        rule_pack_digests: vec!["sha256:fixture".into()],
        rule_pack_bytes: None,
        artifacts: vec![CatalogV3DistributionArtifact {
            url: "https://example.test/pack.json.zst".into(),
            mirror_urls: vec![],
            signature_url: "https://example.test/pack.sigstore.json".into(),
            signature_mirror_urls: vec![],
            digests: vec!["sha256:compressed".into()],
        }],
    };
    CatalogV3RulePackEntry {
        id: "test-pack".into(),
        name: "Test".into(),
        description: "Test pack".into(),
        author: "Community".into(),
        releases: vec![
            release("1.2.0", None),
            release("1.2.1", None),
            release("1.2.2", None),
            release("1.2.3-beta.1", None),
            release("1.2.4", Some("999.0.0")),
            release("1.2.5", Some("invalid")),
            release("1.3.0", None),
            release("2.0.0", None),
        ],
    }
}

#[test]
fn tracked_pack_auto_update_selects_highest_compatible_patch_even_with_newer_major() {
    let pack = pack_fixture();
    let cpu = crate::services::RuntimePerformanceClass::Slow;
    let installed = semver::Version::parse("1.2.0").unwrap();
    let (release, _) = select_rule_pack_candidate(&pack, cpu, None, Some(&installed)).unwrap();
    assert_eq!(release.version, "1.2.2");
    assert_eq!(
        select_rule_pack_candidate(&pack, cpu, None, None)
            .unwrap()
            .0
            .version,
        "2.0.0"
    );
    assert_eq!(
        select_rule_pack_candidate(&pack, cpu, Some("1.2.1"), None)
            .unwrap()
            .0
            .version,
        "1.2.1"
    );
    for version in ["1.2.2", "1.2.3-beta.0", "3.0.0"] {
        assert!(
            select_rule_pack_candidate(
                &pack,
                cpu,
                None,
                Some(&semver::Version::parse(version).unwrap())
            )
            .is_none()
        );
    }
    for version in ["1.2.4", "1.2.5", "unknown"] {
        assert!(select_rule_pack_candidate(&pack, cpu, Some(version), None).is_none());
    }
}

fn manifest_fixture() -> RulePackManifest {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1, "id": "test-pack", "name": "Test", "description": "Test pack", "author": "Community", "version": "1.2.0",
        "rules": [{"id": "recommended", "title": "Recommended", "description": "Recommendations", "category": "anime", "rego_source": "package test\nimport rego.v1\nscore_entry[\"listed\"] := 200", "applied_facets": ["anime"]}]
    })).unwrap()
}

#[test]
fn tracked_pack_manifest_rejects_identity_schema_and_duplicate_template_changes() {
    let pack = pack_fixture();
    let registry = RulePackRegistryEntry::from_release(
        &pack,
        &pack.releases[0],
        &pack.releases[0].artifacts[0],
    );
    assert!(validate_rule_pack_manifest(&manifest_fixture(), &registry).is_ok());
    let mut manifest = manifest_fixture();
    manifest.id = "another-pack".into();
    assert!(validate_rule_pack_manifest(&manifest, &registry).is_err());
    let mut manifest = manifest_fixture();
    manifest.version = "1.2.1".into();
    assert!(validate_rule_pack_manifest(&manifest, &registry).is_err());
    let mut manifest = manifest_fixture();
    manifest.schema_version = 2;
    assert!(validate_rule_pack_manifest(&manifest, &registry).is_err());
    let mut manifest = manifest_fixture();
    manifest.rules.push(manifest.rules[0].clone());
    assert!(validate_rule_pack_manifest(&manifest, &registry).is_err());
    let mut manifest = manifest_fixture();
    manifest.rules[0].rego_source.clear();
    assert!(validate_rule_pack_manifest(&manifest, &registry).is_err());
}
