use super::*;
use quick_xml::NsReader;
use quick_xml::name::ResolveResult;
use scryer_application::IndexerResponseAttributes;

fn result(attributes: Vec<PluginReleaseAttribute>) -> IndexerSearchResult {
    IndexerSearchResult {
        indexer_id: Some("source".into()),
        source: "source".into(),
        title: "Software & tools <edition>".into(),
        link: Some("https://source.example/get?apikey=upstream-secret".into()),
        download_url: Some("https://source.example/get?apikey=upstream-secret".into()),
        source_kind: None,
        size_bytes: Some(123),
        published_at: Some("2026-10-01T00:00:00Z".into()),
        thumbs_up: None,
        thumbs_down: None,
        indexer_languages: None,
        indexer_subtitles: None,
        indexer_grabs: None,
        password_hint: None,
        parsed_release_metadata: None,
        quality_profile_decision: None,
        extra: Default::default(),
        response_attributes: IndexerResponseAttributes {
            original: attributes,
            ..Default::default()
        },
        guid: Some("https://source.example/get?apikey=upstream-secret".into()),
        info_url: None,
        provenance: None,
        candidate_token: None,
        queue_scope: None,
        coverage_scope: None,
        auto_eligible: None,
        auto_decision_code: None,
        auto_decision_summary: None,
        release_listing_json: None,
        http: None,
    }
}

/// The pinned page generation time every test page carries.
fn generated_at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-27T18:30:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn size_attribute() -> (Option<String>, String, String) {
    (Some(TORZNAB_NAMESPACE.into()), "size".into(), "123".into())
}

fn attribute(namespace: Option<&str>, name: &str, value: &str) -> PluginReleaseAttribute {
    PluginReleaseAttribute {
        origin: PluginReleaseAttributeOrigin::Attribute,
        namespace: namespace.map(str::to_owned),
        namespace_uri: None,
        name: name.into(),
        value: value.into(),
    }
}

fn xml(result: &IndexerSearchResult, total: Option<usize>) -> String {
    let mut output = Vec::new();
    write_xml_feed(
        &mut output,
        NabProtocol::Torznab,
        &NabFeedPage {
            offset: 100,
            total,
            generated_at: generated_at(),
        },
        [NabFeedItem {
            result,
            external_guid: "opaque-source-release",
            retrieval_url: "https://scryer.example/torznab/api?t=get&id=reference",
        }],
    )
    .unwrap();
    String::from_utf8(output).unwrap()
}

fn parsed_attributes(xml: &str) -> Vec<(Option<String>, String, String)> {
    let mut reader = NsReader::from_str(xml);
    let mut actual = Vec::new();
    loop {
        match reader.read_resolved_event().unwrap() {
            (namespace, Event::Empty(event)) if event.local_name().as_ref() == "attr" => {
                let namespace = match namespace {
                    ResolveResult::Bound(namespace) => Some(namespace.0.to_string()),
                    ResolveResult::Unbound => None,
                    ResolveResult::Unknown(_) => panic!("unbound prefix in generated XML"),
                };
                let attrs = event
                    .attributes()
                    .map(|value| {
                        let value = value.unwrap();
                        (
                            value.key.as_ref().to_string(),
                            value
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .unwrap()
                                .into_owned(),
                        )
                    })
                    .collect::<std::collections::HashMap<_, _>>();
                actual.push((namespace, attrs["name"].clone(), attrs["value"].clone()));
            }
            (_, Event::Eof) => break,
            _ => {}
        }
    }
    actual
}

#[test]
fn json_wire_metadata_serializes_as_xml_without_losing_lexical_or_repeated_values() {
    // The plugin boundary is JSON even when an upstream uses XML. This proves
    // the writer consumes structured results and has no source-format coupling.
    let wire = r#"[
        {"origin":"attribute","namespace":"newznab","name":"category","value":"6000"},
        {"origin":"attribute","namespace":"newznab","name":"category","value":"6005"},
        {"origin":"attribute","namespace":"torznab","name":"seeders","value":"007"},
        {"origin":"attribute","name":"custom","value":"001.00"},
        {"origin":"attribute","name":"custom","value":"second & <third> \"quoted\""},
        {"origin":"attribute","name":"password","value":"release&password"},
        {"origin":"element","name":"category","value":"Software & tools"}
    ]"#;
    let attributes: Vec<PluginReleaseAttribute> = serde_json::from_str(wire).unwrap();
    let output = xml(&result(attributes), None);
    assert_eq!(
        parsed_attributes(&output),
        vec![
            (
                Some(NEWZNAB_NAMESPACE.into()),
                "category".into(),
                "6000".into()
            ),
            (
                Some(NEWZNAB_NAMESPACE.into()),
                "category".into(),
                "6005".into()
            ),
            (
                Some(TORZNAB_NAMESPACE.into()),
                "seeders".into(),
                "007".into()
            ),
            (None, "custom".into(), "001.00".into()),
            (None, "custom".into(), "second & <third> \"quoted\"".into()),
            (None, "password".into(), "release&password".into()),
            // The source carries no size, so the typed size fills the gap.
            size_attribute(),
        ]
    );
    assert!(output.contains("<category>Software &amp; tools</category>"));
    assert!(output.contains("<title>Software &amp; tools &lt;edition&gt;</title>"));
    assert!(
        !output.contains(":response"),
        "unknown total must stay unknown"
    );
    assert!(!output.contains("upstream-secret"));
}

#[test]
fn namespace_rebinding_is_preserved_per_attribute() {
    let mut first = attribute(Some("custom"), "tag", "one");
    first.namespace_uri = Some("urn:outer".into());
    let mut second = first.clone();
    second.namespace_uri = Some("urn:inner".into());
    let output = xml(&result(vec![first, second]), Some(101));
    assert_eq!(
        parsed_attributes(&output)
            .iter()
            .map(|value| value.0.as_deref())
            .collect::<Vec<_>>(),
        // The typed size follows the source attributes in its own namespace.
        vec![
            Some("urn:outer"),
            Some("urn:inner"),
            Some(TORZNAB_NAMESPACE)
        ]
    );
    assert!(output.contains("offset=\"100\" total=\"101\""));
}

#[test]
fn download_fields_are_replaced_credentials_redacted_and_password_metadata_survives() {
    let output = xml(
        &result(vec![
            attribute(None, "magneturl", "magnet:?xt=urn:btih:123"),
            attribute(None, "downloadurl", "https://source.example/get"),
            attribute(None, "guid", "https://source.example/get?apikey=secret"),
            attribute(
                None,
                "comments",
                "https://source.example/comments?api_key=secret",
            ),
            attribute(
                None,
                "password",
                "https://release.example/?token=release-password",
            ),
            attribute(None, "custom", "keep"),
            attribute(None, "infohash", "012345"),
        ]),
        None,
    );
    let values = parsed_attributes(&output);
    assert_eq!(values.len(), 7);
    assert_eq!(
        values[0].2,
        "https://scryer.example/torznab/api?t=get&id=reference"
    );
    assert_eq!(values[1].2, "opaque-source-release");
    assert_eq!(
        values[2].2,
        "https://source.example/comments?api_key=[redacted]"
    );
    assert_eq!(
        values[3].2,
        "https://release.example/?token=release-password"
    );
    assert_eq!(values[4].2, "keep");
    assert_eq!(values[5].2, "012345");
    assert_eq!(values[6], size_attribute());
}

#[test]
fn invalid_names_and_xml_control_characters_are_skipped_without_truncating_the_feed() {
    for attributes in [
        vec![attribute(Some("bad\"/><injected"), "tag", "value")],
        vec![attribute(Some("undeclared"), "tag", "value")],
        vec![attribute(None, "tag", "bad\0value")],
        vec![PluginReleaseAttribute {
            origin: PluginReleaseAttributeOrigin::Element,
            namespace: None,
            namespace_uri: None,
            name: "category><injected".into(),
            value: "value".into(),
        }],
    ] {
        let result = result(attributes);
        let output = xml(&result, None);
        assert_eq!(parsed_attributes(&output), vec![size_attribute()]);
        assert!(output.ends_with("</channel></rss>"));
        assert!(!output.contains("injected"));
    }
}

#[test]
fn output_failure_is_propagated_without_consuming_the_rest_of_the_page() {
    struct Disconnected;
    impl Write for Disconnected {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let items = std::iter::from_fn(|| -> Option<NabFeedItem<'static>> {
        panic!("output disconnected before rows")
    });
    assert_eq!(
        write_xml_feed(
            Disconnected,
            NabProtocol::Newznab,
            &NabFeedPage {
                offset: 0,
                total: None,
                generated_at: generated_at(),
            },
            items
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn lazy_owned_links_preserve_output_and_stop_after_a_reference_failure() {
    let row = result(Vec::new());
    let page = NabFeedPage {
        offset: 0,
        total: None,
        generated_at: generated_at(),
    };
    for json in [false, true] {
        let mut visited = 0;
        let items = std::iter::from_fn(|| {
            visited += 1;
            match visited {
                1 => Some(Ok(NabFeedItem {
                    result: &row,
                    external_guid: "public-identity".to_owned(),
                    retrieval_url: "https://scryer.example/newznab/api?t=get&id=opaque".to_owned(),
                })),
                2 => Some(Err(io::Error::from(io::ErrorKind::PermissionDenied))),
                _ => panic!("failed reference must stop page serialization"),
            }
        });
        let mut output = Vec::new();
        let error = if json {
            try_write_json_feed(&mut output, NabProtocol::Newznab, &page, items)
        } else {
            try_write_xml_feed(&mut output, NabProtocol::Newznab, &page, items)
        }
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(visited, 2);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("public-identity"));
        assert!(output.contains("id=opaque"));
        assert!(!output.contains("upstream-secret"));
    }
}

#[test]
fn json_output_keeps_repeated_attributes_categories_and_unknown_total() {
    let result = result(vec![
        attribute(Some("newznab"), "category", "6000"),
        attribute(Some("newznab"), "category", "6005"),
        attribute(Some("torznab"), "seeders", "007"),
        attribute(None, "password", "release&password"),
        PluginReleaseAttribute {
            origin: PluginReleaseAttributeOrigin::Element,
            namespace: None,
            namespace_uri: None,
            name: "category".into(),
            value: "Software & tools".into(),
        },
    ]);
    let mut output = Vec::new();
    write_json_feed(
        &mut output,
        NabProtocol::Newznab,
        &NabFeedPage {
            offset: 100,
            total: None,
            generated_at: generated_at(),
        },
        [NabFeedItem {
            result: &result,
            external_guid: "public-id",
            retrieval_url: "https://scryer.example/get",
        }],
    )
    .unwrap();
    let decoded: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let channel = &decoded["channel"];
    assert!(channel.get("newznab:response").is_none());
    let item = &channel["item"][0];
    assert_eq!(item["newznab:attr"][0]["@attributes"]["value"], "6000");
    assert_eq!(item["newznab:attr"][1]["@attributes"]["value"], "6005");
    assert_eq!(item["torznab:attr"][0]["@attributes"]["value"], "007");
    assert_eq!(item["attr"][0]["@attributes"]["value"], "release&password");
    assert_eq!(item["category"][0], "Software & tools");
    assert_eq!(
        item["enclosure"]["@attributes"]["type"],
        "application/x-nzb"
    );
    assert!(
        !String::from_utf8(output)
            .unwrap()
            .contains("upstream-secret")
    );
}

#[test]
fn non_nab_adapters_emit_normalized_claims_without_inventing_extended_attributes() {
    let mut result = result(Vec::new());
    result.response_attributes.categories = vec![
        "4000".into(),
        "6000".into(),
        "100123".into(),
        "Software & tools".into(),
    ];
    result.response_attributes.imdb_id = Some("tt0012345".into());
    result.password_hint = Some("release<&password".into());
    result.indexer_grabs = Some(4);
    result.extra.insert(
        "private-provider-context".into(),
        serde_json::json!("do-not-serialize"),
    );
    let output = xml(&result, Some(1));
    let attributes = parsed_attributes(&output);
    assert_eq!(
        attributes
            .iter()
            .map(|(_, name, value)| (name.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("category", "4000"),
            ("category", "6000"),
            ("category", "100123"),
            ("imdb", "0012345"),
            ("imdbid", "0012345"),
            ("grabs", "4"),
            ("size", "123"),
            ("password", "release<&password"),
        ]
    );
    assert!(output.contains("<category>Software &amp; tools</category>"));
    assert!(!output.contains("do-not-serialize"));
    let mut output = Vec::new();
    write_json_feed(
        &mut output,
        NabProtocol::Torznab,
        &NabFeedPage {
            offset: 0,
            total: Some(1),
            generated_at: generated_at(),
        },
        [NabFeedItem {
            result: &result,
            external_guid: "public-id",
            retrieval_url: "https://scryer.example/get",
        }],
    )
    .unwrap();
    let decoded: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let item = &decoded["channel"]["item"][0];
    assert_eq!(item["torznab:attr"].as_array().unwrap().len(), 8);
    assert_eq!(
        item["torznab:attr"][7]["@attributes"]["value"],
        "release<&password"
    );
    assert_eq!(item["category"][0], "Software & tools");
    assert!(
        !String::from_utf8(output)
            .unwrap()
            .contains("do-not-serialize")
    );
}

#[test]
fn malformed_item_does_not_hide_later_items_or_duplicate_structural_fields() {
    let mut bad = result(vec![]);
    bad.title = "bad\0title".into();
    let good = result(vec![PluginReleaseAttribute {
        origin: PluginReleaseAttributeOrigin::Element,
        namespace: None,
        namespace_uri: None,
        name: "enclosure".into(),
        value: "https://source.example/private".into(),
    }]);
    let mut output = Vec::new();
    write_xml_feed(
        &mut output,
        NabProtocol::Newznab,
        &NabFeedPage {
            offset: 0,
            total: None,
            generated_at: generated_at(),
        },
        [&bad, &good].into_iter().map(|result| NabFeedItem {
            result,
            external_guid: "safe",
            retrieval_url: "https://scryer.example/get",
        }),
    )
    .unwrap();
    let xml = String::from_utf8(output).unwrap();
    assert_eq!(xml.matches("<item>").count(), 1);
    assert_eq!(xml.matches("<enclosure ").count(), 1);
    assert!(xml.ends_with("</channel></rss>"));
    let mut size = size_attribute();
    size.0 = Some(NEWZNAB_NAMESPACE.into());
    assert_eq!(parsed_attributes(&xml), vec![size]);
    let mut output = Vec::new();
    write_json_feed(
        &mut output,
        NabProtocol::Newznab,
        &NabFeedPage {
            offset: 0,
            total: None,
            generated_at: generated_at(),
        },
        [NabFeedItem {
            result: &good,
            external_guid: "safe",
            retrieval_url: "https://scryer.example/get",
        }],
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        json["channel"]["item"][0]["enclosure"]["@attributes"]["url"],
        "https://scryer.example/get"
    );
}

#[test]
fn non_download_urls_keep_their_target_without_credentials() {
    let comments = "https://member:pass@source.example/details/9?apikey=comment-key";
    let mut attributes = vec![
        attribute(Some("newznab"), "comments", comments),
        attribute(None, "details", "https://source.example/key/secret/details"),
        attribute(
            None,
            "infourl",
            "https://other.example/info?token=info-token",
        ),
        attribute(None, "magneturl", "magnet:?xt=urn:btih:abc"),
        attribute(None, "MAGNET", "magnet:?xt=urn:btih:def"),
    ];
    let download_family = [
        "url",
        "link",
        "downloadurl",
        "download_url",
        "nzburl",
        "TorrentUrl",
    ];
    for name in download_family {
        attributes.push(attribute(
            None,
            name,
            "https://source.example/get?apikey=upstream-secret",
        ));
    }
    let output = xml(&result(attributes), None);
    let values = parsed_attributes(&output);
    let retrieval = "https://scryer.example/torznab/api?t=get&id=reference";
    assert_eq!(values.len(), 3 + download_family.len() + 1);
    assert_eq!(values.last(), Some(&size_attribute()));
    assert_eq!(values[0].0.as_deref(), Some(NEWZNAB_NAMESPACE));
    assert_eq!(
        values[0].2,
        scryer_application::url_redaction::redact_url_credentials(comments)
    );
    assert_eq!(values[1].2, "https://source.example/key/secret/details");
    assert_eq!(values[2].2, "https://other.example/info?token=[redacted]");
    for (value, name) in values[3..].iter().zip(download_family) {
        assert_eq!(value.1, name);
        assert_eq!(value.2, retrieval);
    }
    assert!(!output.contains("magnet:"));
    assert!(!output.contains("upstream-secret"));
    assert!(
        output.contains("<link>https://scryer.example/torznab/api?t=get&amp;id=reference</link>")
    );
    assert!(
        output.contains(
            "<enclosure url=\"https://scryer.example/torznab/api?t=get&amp;id=reference\""
        )
    );
}

#[test]
fn xml_and_json_redact_unknown_attributes_and_preserve_archive_passwords() {
    let values = [
        (
            "custom",
            "https://member:login-secret@source.example/a?x=1&api_key=query-secret",
        ),
        (
            "custom",
            "https://source.example/?next=https%3A%2F%2Fother.example%2F%3Ftoken%3Dencoded-secret%26x%3D1",
        ),
        ("custom", "ordinary & metadata"),
        (
            "password",
            "https://archive.example/?token=archive-password",
        ),
    ];
    let source = result(
        values
            .iter()
            .map(|(name, value)| attribute(Some("newznab"), name, value))
            .collect(),
    );
    let xml_output = xml(&source, None);
    let json = json_item(&source);
    let json_output = json.to_string();
    for output in [&xml_output, &json_output] {
        for secret in ["login-secret", "query-secret", "encoded-secret"] {
            assert!(!output.contains(secret), "{output}");
        }
        assert!(output.contains("archive-password"));
    }
    let expected: Vec<_> = values
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                if *name == "password" {
                    value.to_string()
                } else {
                    scryer_application::url_redaction::redact_url_credentials(value)
                },
            )
        })
        .collect();
    assert_eq!(
        parsed_attributes(&xml_output)
            .into_iter()
            .take(4)
            .map(|(_, name, value)| (name, value))
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(json_attributes(&json, "newznab:attr"), expected);
}

fn json_item(result: &IndexerSearchResult) -> serde_json::Value {
    let mut output = Vec::new();
    write_json_feed(
        &mut output,
        NabProtocol::Torznab,
        &NabFeedPage {
            offset: 0,
            total: None,
            generated_at: generated_at(),
        },
        [NabFeedItem {
            result,
            external_guid: "public-id",
            retrieval_url: "https://scryer.example/get",
        }],
    )
    .unwrap();
    let decoded: serde_json::Value = serde_json::from_slice(&output).unwrap();
    decoded["channel"]["item"][0].clone()
}

fn json_attributes(item: &serde_json::Value, key: &str) -> Vec<(String, String)> {
    item[key]
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    let attributes = &value["@attributes"];
                    (
                        attributes["name"].as_str().unwrap().to_string(),
                        attributes["value"].as_str().unwrap().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn typed_torrent() -> IndexerSearchResult {
    let mut result = result(Vec::new());
    result.title = "Synthetic Tracker Release 2026".into();
    result.response_attributes.categories = vec!["5000".into()];
    result.indexer_languages = Some(vec!["English".into()]);
    result.indexer_subtitles = Some(vec!["French".into()]);
    result.indexer_grabs = Some(7);
    result.extra = serde_json::from_value(serde_json::json!({
        "seeders": 12,
        "leechers": 3,
        "info_hash": "0123456789abcdef0123456789abcdef01234567",
        "info_hash_v1": "0123456789abcdef0123456789abcdef01234567",
        "magnet_url": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        "download_volume_factor": 0.0,
        "upload_volume_factor": 1.0,
        "downloadvolumefactor": 0.0,
        "uploadvolumefactor": 1.0,
        "minimum_seed_ratio": 1.5,
        "minimum_seed_time_minutes": 2880,
        "indexer_flags": ["freeleech"],
        "files": "4",
        "year": "2026",
        "poster": "https://images.example/poster.jpg",
        "external_ids": {"imdb_id": "tt0133093", "tvdb_id": "4242"}
    }))
    .unwrap();
    result
}

#[test]
fn xml_and_json_redact_configured_credentials_in_original_and_synthesized_attributes() {
    for parameter in ["key", "access.key"] {
        let url = format!(
            "https://indexer.invalid/details?{parameter}=upstream-secret&id=42&user=reader"
        );
        let mut source = result(vec![
            attribute(Some("torznab"), "unknown-metadata", &url),
            attribute(Some("torznab"), "password", "archive-passphrase"),
        ]);
        source.response_attributes.credential_parameter = Some(parameter.into());
        source.extra.insert("poster".into(), serde_json::json!(url));
        for output in [xml(&source, None), json_item(&source).to_string()] {
            assert!(!output.contains("upstream-secret"), "{output}");
            assert!(output.contains("id=42"), "{output}");
            assert!(output.contains("user=reader"), "{output}");
            assert!(output.contains("archive-passphrase"), "{output}");
            assert!(output.contains("coverurl"), "{output}");
        }
    }
}

#[test]
fn xml_and_json_redact_synthesized_poster_credentials_but_keep_archive_passwords() {
    for poster in [
        "https://member:poster-secret@images.example/poster.jpg?width=600&apikey=poster-key",
        "https://images.example/poster.jpg?width=600&next=https%3A%2F%2Fother.example%2F%3Ftoken%3Dencoded-poster-secret",
    ] {
        let mut source = result(Vec::new());
        source.extra = serde_json::from_value(serde_json::json!({"poster": poster})).unwrap();
        source.password_hint = Some("https://archive.example/?token=archive-passphrase".into());
        let xml_output = xml(&source, None);
        let json = json_item(&source);
        for output in [&xml_output, &json.to_string()] {
            for secret in ["poster-secret", "poster-key", "encoded-poster-secret"] {
                assert!(!output.contains(secret), "{output}");
            }
            assert!(output.contains("width=600"), "{output}");
            assert!(output.contains("archive-passphrase"), "{output}");
        }
        assert!(
            parsed_attributes(&xml_output)
                .iter()
                .any(|(_, name, _)| name == "coverurl")
        );
        assert!(
            json_attributes(&json, "torznab:attr")
                .iter()
                .any(|(name, _)| name == "coverurl")
        );
    }
}

#[test]
fn typed_only_torrent_synthesizes_prowlarr_attributes_in_order() {
    let result = typed_torrent();
    let output = xml(&result, None);
    let expected = [
        ("category", "5000"),
        ("tag", "freeleech"),
        ("language", "English"),
        ("subs", "French"),
        ("tvdbid", "4242"),
        ("imdb", "0133093"),
        ("imdbid", "0133093"),
        ("seeders", "12"),
        ("files", "4"),
        ("grabs", "7"),
        ("peers", "15"),
        ("leechers", "3"),
        ("year", "2026"),
        ("infohash", "0123456789abcdef0123456789abcdef01234567"),
        ("minimumratio", "1.5"),
        ("minimumseedtime", "172800"),
        ("downloadvolumefactor", "0"),
        ("uploadvolumefactor", "1"),
        ("coverurl", "https://images.example/poster.jpg"),
        ("size", "123"),
    ]
    .map(|(name, value)| (name.to_string(), value.to_string()));
    let attributes = parsed_attributes(&output);
    assert!(
        attributes
            .iter()
            .all(|(namespace, _, _)| namespace.as_deref() == Some(TORZNAB_NAMESPACE))
    );
    assert_eq!(
        attributes
            .into_iter()
            .map(|(_, name, value)| (name, value))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        !output.contains("magnet:"),
        "the item link is the only download"
    );
    assert_eq!(
        json_attributes(&json_item(&result), "torznab:attr"),
        expected
    );
}

#[test]
fn source_attributes_win_and_typed_facts_fill_only_the_gaps() {
    let mut result = typed_torrent();
    result.response_attributes.original = vec![
        attribute(Some("torznab"), "seeders", "99"),
        attribute(Some("torznab"), "IMDB", "1234567"),
        attribute(Some("torznab"), "category", "5040"),
    ];
    let output = xml(&result, None);
    let names = parsed_attributes(&output)
        .into_iter()
        .map(|(_, name, value)| (name, value))
        .collect::<Vec<_>>();
    let count = |name: &str| {
        names
            .iter()
            .filter(|(actual, _)| actual.eq_ignore_ascii_case(name))
            .count()
    };
    assert_eq!(count("seeders"), 1);
    assert_eq!(count("imdb"), 1);
    assert_eq!(count("imdbid"), 0, "imdb and imdbid are one identity");
    assert_eq!(count("category"), 1);
    assert_eq!(names[0], ("seeders".into(), "99".into()));
    assert_eq!(names[1], ("IMDB".into(), "1234567".into()));
    for gap in ["peers", "leechers", "infohash", "minimumseedtime", "tvdbid"] {
        assert_eq!(count(gap), 1, "missing synthesized {gap}");
    }
    let item = json_item(&result);
    assert_eq!(json_attributes(&item, "torznab:attr"), names);
}

#[test]
fn missing_or_unparseable_publish_time_uses_the_page_generation_time() {
    let expected = generated_at().to_rfc2822();
    for published_at in [None, Some("not a date".to_string())] {
        let mut result = result(Vec::new());
        result.published_at = published_at;
        let output = xml(&result, None);
        assert!(
            output.contains(&format!("<pubDate>{expected}</pubDate>")),
            "{output}"
        );
        assert_eq!(json_item(&result)["pubDate"], expected.as_str());
    }
    let dated = result(Vec::new());
    assert!(xml(&dated, None).contains("<pubDate>Thu, 1 Oct 2026 00:00:00 +0000</pubDate>"));
}
