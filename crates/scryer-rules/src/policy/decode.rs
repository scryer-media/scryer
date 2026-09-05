//! Bounded decoders for the open-ended lists a policy may emit.
//!
//! Reason codes and tags are persisted, indexed, and rendered, so an unbounded
//! list is a storage and display hazard rather than a useful signal. Both
//! decoders fail closed: anything that is not exactly the declared shape is an
//! error for that rule, never a coerced value.

use regorus::Value;

/// Bounds on the reason codes a rule may emit.
pub const MAX_REASON_CODES: usize = 32;
pub const MAX_REASON_CODE_LEN: usize = 120;

/// Bounds on the tags a rule may emit.
pub const MAX_TAGS: usize = 16;
pub const MAX_TAG_LEN: usize = 64;

/// Namespace reserved for tags Scryer itself applies. A policy that could mint
/// one would be able to forge host provenance, so the prefix is refused rather
/// than silently rewritten.
pub const RESERVED_TAG_PREFIX: &str = "scryer:";

/// Collect the string items of an array or set rule, failing on anything else.
fn string_items<'a>(value: &'a Value, field: &str) -> Result<Vec<&'a Value>, String> {
    if let Ok(array) = value.as_array() {
        Ok(array.iter().collect())
    } else if let Ok(set) = value.as_set() {
        Ok(set.iter().collect())
    } else {
        Err(format!("'{field}' must be an array or set of strings"))
    }
}

/// Decode a policy's `reasons` output into bounded stable machine codes.
pub fn decode_reasons(value: &Value) -> Result<Vec<String>, String> {
    let items = string_items(value, "reasons")?;

    if items.len() > MAX_REASON_CODES {
        return Err(format!(
            "'reasons' has {} entries, at most {MAX_REASON_CODES} are allowed",
            items.len()
        ));
    }

    let mut reasons = Vec::with_capacity(items.len());
    for item in items {
        let reason = item
            .as_string()
            .map_err(|_| "'reasons' must contain only strings".to_string())?;
        if reason.len() > MAX_REASON_CODE_LEN {
            return Err(format!(
                "reason code is {} characters, at most {MAX_REASON_CODE_LEN} are allowed",
                reason.len()
            ));
        }
        reasons.push(reason.to_string());
    }

    Ok(reasons)
}

/// Decode a policy's `tags` output.
///
/// Tags are applied to real library entities, so they are held to the shape
/// Scryer's own tag vocabulary uses: printable ASCII words, separators, and
/// nothing from the reserved host namespace. Order is the order the rule
/// produced (a Rego set is already sorted); duplicates are dropped so a tag a
/// rule derives twice is applied once.
pub fn decode_tags(value: &Value) -> Result<Vec<String>, String> {
    let items = string_items(value, "tags")?;

    if items.len() > MAX_TAGS {
        return Err(format!(
            "'tags' has {} entries, at most {MAX_TAGS} are allowed",
            items.len()
        ));
    }

    let mut tags: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let tag = item
            .as_string()
            .map_err(|_| "'tags' must contain only strings".to_string())?
            .to_string();
        if tag.is_empty() {
            return Err("tag must not be empty".to_string());
        }
        if tag.len() > MAX_TAG_LEN {
            return Err(format!(
                "tag is {} characters, at most {MAX_TAG_LEN} are allowed",
                tag.len()
            ));
        }
        if tag.starts_with(RESERVED_TAG_PREFIX) {
            return Err(format!(
                "tag '{tag}' uses the reserved '{RESERVED_TAG_PREFIX}' prefix"
            ));
        }
        if let Some(bad) = tag
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ' ' | '-')))
        {
            return Err(format!(
                "tag '{tag}' contains unsupported character '{bad}'; use letters, digits, \
                 '.', '_', '-', or spaces"
            ));
        }
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }

    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn array(items: &[&str]) -> Value {
        Value::from(
            items
                .iter()
                .map(|item| Value::from(item.to_string()))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn tags_decode_from_an_array_in_order() {
        let tags = decode_tags(&array(&["kids", "family movie", "seen-2024", "v1.0"]))
            .expect("well-formed tags");
        assert_eq!(tags, vec!["kids", "family movie", "seen-2024", "v1.0"]);
    }

    #[test]
    fn tags_decode_from_a_set() {
        let mut set = std::collections::BTreeSet::new();
        set.insert(Value::from("beta".to_string()));
        set.insert(Value::from("alpha".to_string()));
        let tags = decode_tags(&Value::from(set)).expect("well-formed tags");
        assert_eq!(tags, vec!["alpha", "beta"], "a Rego set is already sorted");
    }

    #[test]
    fn duplicate_tags_are_applied_once() {
        let tags = decode_tags(&array(&["kids", "kids"])).expect("well-formed tags");
        assert_eq!(tags, vec!["kids"]);
    }

    #[test]
    fn oversized_tag_list_is_rejected() {
        let owned: Vec<String> = (0..=MAX_TAGS).map(|i| format!("tag{i}")).collect();
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        let error = decode_tags(&array(&borrowed)).expect_err("too many tags");
        assert!(error.contains("at most 16"), "{error}");
    }

    #[test]
    fn overlong_tag_is_rejected() {
        let long = "x".repeat(MAX_TAG_LEN + 1);
        let error = decode_tags(&array(&[&long])).expect_err("tag too long");
        assert!(error.contains("at most 64"), "{error}");
    }

    #[test]
    fn reserved_prefix_is_rejected_so_host_provenance_cannot_be_forged() {
        let error = decode_tags(&array(&["scryer:managed"])).expect_err("reserved prefix");
        assert!(error.contains("reserved"), "{error}");
    }

    #[test]
    fn unsupported_characters_are_rejected() {
        for tag in ["kids/teens", "kids\ttab", "kids\u{1f600}", "kids:teens"] {
            let error = decode_tags(&array(&[tag])).expect_err("{tag} should be rejected");
            assert!(error.contains("unsupported character"), "{error}");
        }
    }

    #[test]
    fn empty_tag_is_rejected() {
        let error = decode_tags(&array(&[""])).expect_err("empty tag");
        assert!(error.contains("must not be empty"), "{error}");
    }

    #[test]
    fn non_list_tags_are_rejected() {
        let error = decode_tags(&Value::from("kids".to_string())).expect_err("scalar tags");
        assert!(error.contains("array or set"), "{error}");
    }

    #[test]
    fn non_string_tag_entries_are_rejected() {
        let error = decode_tags(&Value::from(vec![Value::from(1i64)])).expect_err("numeric tag");
        assert!(error.contains("only strings"), "{error}");
    }

    #[test]
    fn reasons_keep_their_existing_bounds() {
        assert_eq!(
            decode_reasons(&array(&["stale", "unwatched"])).expect("well-formed"),
            vec!["stale", "unwatched"]
        );
        let owned: Vec<String> = (0..=MAX_REASON_CODES).map(|i| format!("r{i}")).collect();
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        let error = decode_reasons(&array(&borrowed)).expect_err("too many reasons");
        assert!(error.contains("at most 32"), "{error}");
    }
}
