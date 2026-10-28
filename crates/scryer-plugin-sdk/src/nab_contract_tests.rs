use super::*;

#[test]
fn legacy_search_payloads_default_additive_nab_fields() {
    let request: PluginSearchRequest =
        serde_json::from_value(serde_json::json!({"query":"Fixture"})).unwrap();
    assert!(!request.recent_paging);
    assert!(request.host_caps.is_none());
    assert!(request.nab.is_none());
    let result: PluginSearchResult =
        serde_json::from_value(serde_json::json!({"title":"Fixture"})).unwrap();
    assert!(result.attributes.is_empty());
    let capabilities: IndexerCapabilities = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(!capabilities.raw_nab_search);
}

#[test]
fn additive_nab_contract_preserves_operations_caps_and_original_attributes() {
    let request = PluginSearchRequest {
        nab: Some(PluginNabSearch {
            operation: PluginNabOperation::Book,
            parameters: [
                ("author".to_owned(), "A & B".to_owned()),
                ("title".to_owned(), "Fixture".to_owned()),
            ]
            .into(),
        }),
        host_caps: Some(PluginSearchCaps {
            searching_present: true,
            book_search: Some(PluginCapsSearchNode {
                available: true,
                supported_params: vec!["author".into(), "title".into()],
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let encoded = serde_json::to_value(&request).unwrap();
    assert_eq!(encoded["nab"]["operation"], "book");
    assert_eq!(encoded["nab"]["parameters"]["author"], "A & B");
    let decoded: PluginSearchRequest = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.nab, request.nab);
    assert_eq!(decoded.host_caps, request.host_caps);
    let attributes = vec![
        PluginReleaseAttribute {
            origin: PluginReleaseAttributeOrigin::Attribute,
            namespace: Some("n".into()),
            namespace_uri: Some("urn:fixture:source".into()),
            name: "custom".into(),
            value: "007".into(),
        },
        PluginReleaseAttribute {
            origin: PluginReleaseAttributeOrigin::Attribute,
            namespace: Some("n".into()),
            namespace_uri: Some("urn:fixture:source".into()),
            name: "custom".into(),
            value: "001.00".into(),
        },
    ];
    let result = PluginSearchResult {
        title: "Fixture".into(),
        attributes: attributes.clone(),
        ..Default::default()
    };
    let decoded: PluginSearchResult =
        serde_json::from_slice(&serde_json::to_vec(&result).unwrap()).unwrap();
    assert_eq!(decoded.attributes, attributes);
}
