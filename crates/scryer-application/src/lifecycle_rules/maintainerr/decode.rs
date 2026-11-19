//! Reads Maintainerr's wire documents into plain structures.
//!
//! The documents are read leniently where Maintainerr itself is lenient:
//! numbers may arrive as numeric strings, a rule may arrive as the entity
//! (with its JSON in `ruleJson`) or as the rule DTO itself, and the group's
//! collection settings may sit at the top level or inside `collection`.

use super::{MAX_MAINTAINERR_RULE_GROUPS, MaintainerrImportRefusal};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// The level a group's media is evaluated at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MediaLevel {
    Movie,
    Show,
    Season,
    Episode,
}

impl MediaLevel {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Movie => "movies",
            Self::Show => "shows",
            Self::Season => "seasons",
            Self::Episode => "episodes",
        }
    }

    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
                "movie" | "1" => Some(Self::Movie),
                "show" | "2" => Some(Self::Show),
                "season" | "3" => Some(Self::Season),
                "episode" | "4" => Some(Self::Episode),
                _ => None,
            },
            Value::Number(_) => match as_i64(value)? {
                1 => Some(Self::Movie),
                2 => Some(Self::Show),
                3 => Some(Self::Season),
                4 => Some(Self::Episode),
                _ => None,
            },
            _ => None,
        }
    }
}

/// One property of the instance's catalogue.
#[derive(Debug, Clone)]
pub(super) struct CatalogueProperty {
    pub app_name: String,
    pub name: String,
    pub human_name: Option<String>,
    /// Maintainerr's rule type key: `0` number, `1` date, `2` text, `3`
    /// boolean, `4` text list.
    pub type_key: String,
}

/// The instance's property catalogue.
#[derive(Debug, Clone, Default)]
pub(super) struct Catalogue {
    /// Keyed by Maintainerr's (application id, property id).
    pub properties: HashMap<(i64, i64), CatalogueProperty>,
    /// The ids of the applications the catalogue lists.
    pub applications: HashSet<i64>,
}

pub(super) fn decode_catalogue(json: &str) -> Result<Catalogue, MaintainerrImportRefusal> {
    let root: Value =
        serde_json::from_str(json).map_err(|_| MaintainerrImportRefusal::ConstantsJsonInvalid)?;
    decode_catalogue_value(&root)
}

pub(super) fn decode_catalogue_value(root: &Value) -> Result<Catalogue, MaintainerrImportRefusal> {
    let invalid = MaintainerrImportRefusal::ConstantsJsonInvalid;
    let applications = root
        .get("applications")
        .and_then(Value::as_array)
        .ok_or(invalid)?;
    let mut catalogue = Catalogue::default();
    for application in applications {
        let app_id = application.get("id").and_then(as_i64).ok_or(invalid)?;
        catalogue.applications.insert(app_id);
        let app_name = application
            .get("name")
            .and_then(Value::as_str)
            .ok_or(invalid)?
            .to_string();
        let props = application
            .get("props")
            .and_then(Value::as_array)
            .ok_or(invalid)?;
        for prop in props {
            let prop_id = prop.get("id").and_then(as_i64).ok_or(invalid)?;
            let name = prop
                .get("name")
                .and_then(Value::as_str)
                .ok_or(invalid)?
                .to_string();
            let type_key = prop
                .get("type")
                .and_then(|kind| kind.get("key"))
                .and_then(as_key)
                .ok_or(invalid)?;
            let human_name = prop
                .get("humanName")
                .and_then(Value::as_str)
                .map(str::to_string);
            catalogue.properties.insert(
                (app_id, prop_id),
                CatalogueProperty {
                    app_name: app_name.clone(),
                    name,
                    human_name,
                    type_key,
                },
            );
        }
    }
    Ok(catalogue)
}

/// The collection settings of a group that decide what happens to matches.
#[derive(Debug, Clone, Default)]
pub(super) struct CollectionSettings {
    pub arr_action: Option<i64>,
    pub delete_after_days: Option<f64>,
    pub radarr_settings_id: Option<i64>,
    pub sonarr_settings_id: Option<i64>,
    pub radarr_quality_profile_id: Option<i64>,
    pub sonarr_quality_profile_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Operator {
    And,
    Or,
}

#[derive(Debug, Clone)]
pub(super) struct CustomValue {
    pub rule_type_id: i64,
    pub value: String,
}

#[derive(Debug, Clone)]
pub(super) struct DecodedRule {
    pub operator: Option<Operator>,
    pub action: i64,
    pub first: (i64, i64),
    pub last: Option<(i64, i64)>,
    pub custom: Option<CustomValue>,
    pub username: Option<String>,
    pub section: i64,
}

#[derive(Debug, Clone)]
pub(super) struct DecodedGroup {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub library_id: Option<String>,
    pub is_active: bool,
    pub use_rules: bool,
    pub level: Option<MediaLevel>,
    pub collection: CollectionSettings,
    /// The rules, or the zero-based index of the first one that is malformed
    /// and why.
    pub rules: Result<Vec<DecodedRule>, (usize, String)>,
}

pub(super) fn decode_groups(json: &str) -> Result<Vec<DecodedGroup>, MaintainerrImportRefusal> {
    let invalid = MaintainerrImportRefusal::GroupsJsonInvalid;
    let root: Value = serde_json::from_str(json).map_err(|_| invalid)?;
    decode_group_values(root.as_array().ok_or(invalid)?)
}

pub(super) fn decode_group_values(
    groups: &[Value],
) -> Result<Vec<DecodedGroup>, MaintainerrImportRefusal> {
    let invalid = MaintainerrImportRefusal::GroupsJsonInvalid;
    if groups.len() > MAX_MAINTAINERR_RULE_GROUPS {
        return Err(MaintainerrImportRefusal::GroupsTooMany);
    }
    groups
        .iter()
        .map(|group| decode_group(group.as_object().ok_or(invalid)?).ok_or(invalid))
        .collect()
}

fn decode_group(group: &Map<String, Value>) -> Option<DecodedGroup> {
    let id = group.get("id").and_then(as_i64)?;
    let collection = group.get("collection").and_then(Value::as_object);
    // Maintainerr serves these on the collection; a group-level copy is
    // read only when the collection has none.
    let setting = |key: &str| -> Option<&Value> {
        collection
            .and_then(|c| c.get(key))
            .filter(|value| !value.is_null())
            .or_else(|| group.get(key))
            .filter(|value| !value.is_null())
    };
    let level = group
        .get("dataType")
        .filter(|value| !value.is_null())
        .or_else(|| collection.and_then(|c| c.get("type")))
        .and_then(MediaLevel::from_value);
    let rules = match group.get("rules") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(rules)) => {
            // Maintainerr evaluates a group's rules in row id order.
            let mut ordered: Vec<&Value> = rules.iter().collect();
            if ordered
                .iter()
                .all(|rule| rule.get("id").and_then(as_i64).is_some())
            {
                ordered.sort_by_key(|rule| rule.get("id").and_then(as_i64));
            }
            ordered
                .into_iter()
                .enumerate()
                .map(|(index, rule)| decode_rule(rule).map_err(|why| (index, why)))
                .collect()
        }
        Some(_) => Err((0, "the group's rules are not a list".to_string())),
    };
    Some(DecodedGroup {
        id,
        name: group
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        description: group
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        library_id: group.get("libraryId").and_then(as_text),
        is_active: group
            .get("isActive")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        use_rules: group
            .get("useRules")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        level,
        collection: CollectionSettings {
            arr_action: setting("arrAction").and_then(as_i64),
            delete_after_days: setting("deleteAfterDays").and_then(as_f64),
            radarr_settings_id: setting("radarrSettingsId").and_then(as_i64),
            sonarr_settings_id: setting("sonarrSettingsId").and_then(as_i64),
            radarr_quality_profile_id: setting("radarrQualityProfileId").and_then(as_i64),
            sonarr_quality_profile_id: setting("sonarrQualityProfileId").and_then(as_i64),
        },
        rules,
    })
}

fn decode_rule(entry: &Value) -> Result<DecodedRule, String> {
    let entry = entry
        .as_object()
        .ok_or_else(|| "the rule is not an object".to_string())?;
    let parsed;
    let dto = match entry.get("ruleJson") {
        Some(Value::String(text)) => {
            parsed = serde_json::from_str::<Value>(text)
                .map_err(|_| "the rule's ruleJson is not valid JSON".to_string())?;
            parsed
                .as_object()
                .ok_or_else(|| "the rule's ruleJson is not an object".to_string())?
        }
        Some(Value::Object(object)) => object,
        _ => entry,
    };
    let pair = |key: &str| -> Result<Option<(i64, i64)>, String> {
        match dto.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) if items.len() == 2 => {
                match (as_i64(&items[0]), as_i64(&items[1])) {
                    (Some(app), Some(prop)) => Ok(Some((app, prop))),
                    _ => Err(format!(
                        "the rule's {key} is not an application and property id"
                    )),
                }
            }
            Some(_) => Err(format!(
                "the rule's {key} is not an application and property id"
            )),
        }
    };
    let first = pair("firstVal")?.ok_or_else(|| "the rule has no firstVal".to_string())?;
    let last = pair("lastVal")?;
    let operator = match dto.get("operator") {
        None | Some(Value::Null) => None,
        Some(value) => match as_i64(value) {
            Some(0) => Some(Operator::And),
            Some(1) => Some(Operator::Or),
            _ => return Err("the rule's operator is neither AND nor OR".to_string()),
        },
    };
    let action = dto
        .get("action")
        .and_then(as_i64)
        .ok_or_else(|| "the rule has no comparison".to_string())?;
    let custom = match dto.get("customVal") {
        None | Some(Value::Null) => None,
        Some(Value::Object(custom)) => {
            let rule_type_id = custom
                .get("ruleTypeId")
                .and_then(as_i64)
                .ok_or_else(|| "the rule's custom value has no type".to_string())?;
            let value = custom
                .get("value")
                .and_then(as_text)
                .ok_or_else(|| "the rule's custom value has no value".to_string())?;
            Some(CustomValue {
                rule_type_id,
                value,
            })
        }
        Some(_) => return Err("the rule's custom value is not an object".to_string()),
    };
    // Maintainerr reads the section of the stored rule, not of its JSON.
    let section = entry
        .get("section")
        .filter(|value| !value.is_null())
        .or_else(|| dto.get("section"))
        .and_then(as_i64)
        .unwrap_or(0);
    let username = dto
        .get("username")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    Ok(DecodedRule {
        operator,
        action,
        first,
        last,
        custom,
        username,
        section,
    })
}

/// An integer, or a string holding one.
fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|float| float.fract() == 0.0 && float.abs() < 9.0e15)
                .map(|float| float as i64)
        }),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
    .filter(|float: &f64| float.is_finite())
}

fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn as_key(value: &Value) -> Option<String> {
    as_text(value).map(|key| key.trim().to_string())
}
