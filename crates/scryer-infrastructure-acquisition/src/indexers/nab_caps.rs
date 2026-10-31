//! Compatibility discovery and error documents. No upstream access lives here.

use quick_xml::{
    Writer,
    events::{BytesDecl, BytesEnd, BytesStart, Event},
};
use scryer_domain::{IndexerCapsSearchNode, IndexerCapsSnapshot, IndexerCategoryValueKind};
use serde::Serialize;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{self, Write},
};

#[derive(Serialize)]
struct CategoryAttributes<'a> {
    id: u32,
    name: Cow<'a, str>,
}

#[derive(Serialize)]
struct Category<'a> {
    #[serde(rename = "@attributes")]
    attributes: CategoryAttributes<'a>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    subcat: Vec<Category<'a>>,
}

fn category_tree(caps: &IndexerCapsSnapshot) -> Vec<Category<'_>> {
    let mut flat = BTreeMap::new();
    for category in &caps.categories.categories {
        if category.value_kind != IndexerCategoryValueKind::Numeric
            || category.value.is_empty()
            || !category.value.bytes().all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        // Sonarr and Radarr parse these with System.Int32.Parse.
        let Ok(id) = category.value.parse::<i32>() else {
            continue;
        };
        flat.entry(id as u32).or_insert(
            category
                .label
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(&category.value),
        );
    }
    let mut tree = BTreeMap::<u32, Category<'_>>::new();
    for (&id, &name) in &flat {
        let parent = if (1000..10000).contains(&id) {
            id / 1000 * 1000
        } else {
            id
        };
        let root = tree.entry(parent).or_insert_with(|| Category {
            attributes: CategoryAttributes {
                id: parent,
                name: flat.get(&parent).map_or_else(
                    || Cow::Owned(parent.to_string()),
                    |name| Cow::Borrowed(*name),
                ),
            },
            subcat: Vec::new(),
        });
        if id != parent {
            root.subcat.push(Category {
                attributes: CategoryAttributes {
                    id,
                    name: Cow::Borrowed(name),
                },
                subcat: Vec::new(),
            });
        }
    }
    tree.into_values().collect()
}

fn modes(caps: &IndexerCapsSnapshot) -> [(&str, Option<&IndexerCapsSearchNode>); 5] {
    [
        ("search", caps.search.as_ref()),
        ("tv-search", caps.tv_search.as_ref()),
        ("movie-search", caps.movie_search.as_ref()),
        (
            "music-search",
            caps.music_search.as_ref().or(caps.audio_search.as_ref()),
        ),
        ("book-search", caps.book_search.as_ref()),
    ]
}

fn limits(caps: &IndexerCapsSnapshot) -> io::Result<(i64, i64)> {
    let default = caps
        .limits_default
        .unwrap_or(scryer_application::RAW_SEARCH_PAGE_SIZE as i64);
    let max = caps
        .limits_max
        .unwrap_or(scryer_application::RAW_SEARCH_PAGE_SIZE as i64);
    let default = default.clamp(1, i32::MAX as i64);
    let max = max.clamp(1, i32::MAX as i64).max(default);
    Ok((default, max))
}

pub fn write_xml_caps(output: impl Write, caps: &IndexerCapsSnapshot) -> io::Result<()> {
    let mut writer = Writer::new(output);
    let (default, max) = limits(caps)?;
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    writer.write_event(Event::Start(BytesStart::new("caps")))?;
    writer
        .create_element("server")
        .with_attribute(("title", "Scryer"))
        .write_empty()?;
    writer
        .create_element("limits")
        .with_attributes([
            ("default", default.to_string().as_str()),
            ("max", max.to_string().as_str()),
        ])
        .write_empty()?;
    writer
        .create_element("registration")
        .with_attributes([("available", "no"), ("open", "no")])
        .write_empty()?;
    writer.write_event(Event::Start(BytesStart::new("searching")))?;
    for (name, node) in modes(caps) {
        let available = node.is_some_and(|node| node.available);
        let params = node
            .filter(|_| available)
            .map(|node| node.supported_params.join(","))
            .unwrap_or_default();
        writer
            .create_element(name)
            .with_attributes([
                ("available", if available { "yes" } else { "no" }),
                ("supportedParams", params.as_str()),
            ])
            .write_empty()?;
    }
    writer.write_event(Event::End(BytesEnd::new("searching")))?;
    writer.write_event(Event::Start(BytesStart::new("categories")))?;
    for category in category_tree(caps) {
        let mut tag = BytesStart::new("category");
        let id = category.attributes.id.to_string();
        tag.push_attribute(("id", id.as_str()));
        tag.push_attribute(("name", category.attributes.name.as_ref()));
        writer.write_event(Event::Start(tag))?;
        for child in category.subcat {
            writer
                .create_element("subcat")
                .with_attributes([
                    ("id", child.attributes.id.to_string().as_str()),
                    ("name", child.attributes.name.as_ref()),
                ])
                .write_empty()?;
        }
        writer.write_event(Event::End(BytesEnd::new("category")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("categories")))?;
    writer.write_event(Event::End(BytesEnd::new("caps")))?;
    Ok(())
}

pub fn write_json_caps(mut output: impl Write, caps: &IndexerCapsSnapshot) -> io::Result<()> {
    let (default, max) = limits(caps)?;
    let searching = modes(caps).map(|(name, node)| {
        let available = node.is_some_and(|node| node.available);
        (name, serde_json::json!({"@attributes": {"available": if available { "yes" } else { "no" },
            "supportedParams": node.filter(|_| available).map(|node| node.supported_params.join(",")).unwrap_or_default()}}))
    }).into_iter().collect::<BTreeMap<_, _>>();
    // Stream the category tree from borrowed labels rather than cloning all
    // source metadata into an intermediate JSON value.
    output.write_all(b"{\"caps\":{")?;
    serde_json::to_writer(&mut output, "server")?;
    output.write_all(b":")?;
    serde_json::to_writer(
        &mut output,
        &serde_json::json!({"@attributes": {"title": "Scryer"}}),
    )?;
    output.write_all(b",\"limits\":")?;
    serde_json::to_writer(
        &mut output,
        &serde_json::json!({"@attributes": {"default": default, "max": max}}),
    )?;
    output.write_all(b",\"registration\":{\"@attributes\":{\"available\":\"no\",\"open\":\"no\"}},\"searching\":")?;
    serde_json::to_writer(&mut output, &searching)?;
    output.write_all(b",\"categories\":{\"category\":")?;
    serde_json::to_writer(&mut output, &category_tree(caps))?;
    output.write_all(b"}}}")
}

pub fn write_xml_error(output: impl Write, code: u16, description: &str) -> io::Result<()> {
    let mut writer = Writer::new(output);
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    writer
        .create_element("error")
        .with_attributes([
            ("code", code.to_string().as_str()),
            ("description", description),
        ])
        .write_empty()?;
    Ok(())
}

pub fn write_json_error(output: impl Write, code: u16, description: &str) -> io::Result<()> {
    serde_json::to_writer(
        output,
        &serde_json::json!({"error": {"@attributes": {"code": code, "description": description}}}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::IndexerCategoryDescriptor;

    #[test]
    fn caps_documents_have_explicit_modes_integer_limits_and_unique_numeric_tree() {
        let mut caps = IndexerCapsSnapshot::default();
        for (value, label, kind) in [
            ("4000", "Software", IndexerCategoryValueKind::Numeric),
            (
                "4010",
                "Tools & utilities",
                IndexerCategoryValueKind::Numeric,
            ),
            ("4010", "Duplicate", IndexerCategoryValueKind::Numeric),
            ("100123", "Custom", IndexerCategoryValueKind::Numeric),
            (
                "100124",
                "Another custom",
                IndexerCategoryValueKind::Numeric,
            ),
            ("6000", "Adult", IndexerCategoryValueKind::Numeric),
            (
                "4294967295",
                "Too large for consumers",
                IndexerCategoryValueKind::Numeric,
            ),
            ("7000", "String code", IndexerCategoryValueKind::String),
        ] {
            caps.categories.categories.push(IndexerCategoryDescriptor {
                value: value.into(),
                label: Some(label.into()),
                value_kind: kind,
                facets: Vec::new(),
            });
        }
        let mut xml = Vec::new();
        write_xml_caps(&mut xml, &caps).unwrap();
        let xml = String::from_utf8(xml).unwrap();
        assert!(xml.contains("<limits default=\"100\" max=\"100\""));
        for (name, _) in modes(&caps) {
            assert!(xml.contains(&format!("<{name} available=\"no\" supportedParams=\"\"")));
        }
        assert_eq!(xml.matches("id=\"4010\"").count(), 1);
        assert!(xml.contains("<subcat id=\"4010\" name=\"Tools &amp; utilities\""));
        assert!(xml.contains("<category id=\"100123\""));
        assert!(xml.contains("<category id=\"100124\""));
        assert!(!xml.contains("4294967295") && !xml.contains("7000"));
        let mut json = Vec::new();
        write_json_caps(&mut json, &caps).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(json["caps"]["limits"]["@attributes"]["max"], 100);
        assert_eq!(
            json["caps"]["categories"]["category"][0]["subcat"][0]["@attributes"]["id"],
            4010
        );
    }
    #[test]
    fn malformed_limits_are_clamped_without_losing_caps() {
        for (default, maximum, expected) in [
            (0, -1, (1, 1)),
            (100, 5, (100, 100)),
            (i64::MAX, i64::MAX, (i32::MAX as i64, i32::MAX as i64)),
        ] {
            let caps = IndexerCapsSnapshot {
                limits_default: Some(default),
                limits_max: Some(maximum),
                ..Default::default()
            };
            assert_eq!(limits(&caps).unwrap(), expected);
            write_xml_caps(io::sink(), &caps).unwrap();
        }
    }
}
