//! Parses and normalizes pasted Sonarr or Radarr custom-format JSON.
//!
//! Lookups follow the JavaScript object semantics the exports were written
//! against: keys enumerate with array indexes first, and property names a
//! plain object inherits count as already defined.

use super::codes;
use super::js::{OBJECT_PROTOTYPE_NAMES, js_number_to_string, js_trim, object_order};
use super::{
    ArrDiagnostic, ArrDiagnosticSeverity, ArrImportRefusal, ArrSourceApplication,
    MAX_PASTED_JSON_BYTES,
};
use serde_json::{Map, Value};

pub(super) const MAX_FORMATS: usize = 1_000;
pub(super) const MAX_SPECIFICATIONS: usize = 1_000;

/// One normalized specification. `fields` keeps the pasted values.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Specification {
    pub(super) implementation: String,
    pub(super) name: String,
    pub(super) negate: bool,
    pub(super) required: bool,
    pub(super) fields: Vec<(String, Value)>,
    pub(super) index: usize,
}

impl Specification {
    pub(super) fn field(&self, name: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ImportedFormat {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) source: ArrSourceApplication,
    pub(super) specifications: Vec<Specification>,
    pub(super) suggested_scores: Vec<(String, f64)>,
    pub(super) description: Option<String>,
    pub(super) inspection_diagnostics: Vec<ArrDiagnostic>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Inspection {
    pub(super) formats: Vec<ImportedFormat>,
    pub(super) diagnostics: Vec<ArrDiagnostic>,
}

fn error(code: &'static str, message: impl Into<String>, location: String) -> ArrDiagnostic {
    ArrDiagnostic {
        code,
        severity: ArrDiagnosticSeverity::Error,
        message: message.into(),
        location: Some(location),
        format_id: None,
        specification_index: None,
    }
}

/// A non-blank string, trimmed.
fn string_value(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => {
            let trimmed = js_trim(text);
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => None,
    }
}

fn number_value(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

/// `a ?? b`: JSON `null` and a missing key both fall through.
fn first_present<'a>(raw: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| raw.get(*key))
        .find(|value| !value.is_null())
}

fn normalize_fields(value: Option<&Value>) -> Option<Vec<(String, Value)>> {
    match value? {
        Value::Object(map) => Some(object_order(
            map.iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )),
        Value::Array(entries) => {
            let mut fields: Vec<(String, Value)> = Vec::new();
            for entry in entries {
                let Value::Object(entry) = entry else {
                    return None;
                };
                let name =
                    string_value(entry.get("name")).or_else(|| string_value(entry.get("key")));
                let (Some(name), Some(value)) = (name, entry.get("value")) else {
                    return None;
                };
                let inherited = OBJECT_PROTOTYPE_NAMES.contains(&name.as_str());
                if inherited || fields.iter().any(|(key, _)| *key == name) {
                    return None;
                }
                fields.push((name, value.clone()));
            }
            Some(object_order(fields))
        }
        _ => None,
    }
}

fn set_score(scores: &mut Vec<(String, f64)>, name: &str, value: f64) {
    // Assigning `__proto__` on a plain object never creates a key.
    if name == "__proto__" {
        return;
    }
    match scores.iter_mut().find(|(key, _)| key == name) {
        Some(entry) => entry.1 = value,
        None => scores.push((name.to_string(), value)),
    }
}

fn normalize_scores(
    raw: &Map<String, Value>,
    diagnostics: &mut Vec<ArrDiagnostic>,
    location: String,
) -> Vec<(String, f64)> {
    let mut result = Vec::new();
    if let Some(Value::Object(candidate)) =
        first_present(raw, &["trash_scores", "trashScores", "scores"])
    {
        let entries = object_order(
            candidate
                .iter()
                .map(|(key, value)| (key.clone(), value))
                .collect(),
        );
        for (name, score) in entries {
            if let Some(numeric) = number_value(Some(score)) {
                set_score(&mut result, &name, numeric);
            }
        }
    }
    if let Some(direct) = number_value(raw.get("score")) {
        let existing = result
            .iter()
            .find(|(key, _)| key == "default")
            .map(|(_, value)| *value);
        if existing.is_some_and(|existing| existing != direct) {
            diagnostics.push(ArrDiagnostic {
                severity: ArrDiagnosticSeverity::Warning,
                ..error(
                    codes::SCORE_CONFLICT,
                    "The exported score conflicts with trash_scores.default; choose a score explicitly.",
                    location,
                )
            });
            set_score(&mut result, "exported score", direct);
        } else {
            set_score(&mut result, "default", direct);
        }
    }
    object_order(result)
}

enum Normalized {
    Format(ImportedFormat),
    Refused(ArrDiagnostic),
}

fn normalize_format(raw: &Value, source: ArrSourceApplication, location: &str) -> Normalized {
    let Value::Object(raw) = raw else {
        return Normalized::Refused(error(
            codes::FORMAT_INVALID,
            "A custom format must be an object.",
            location.to_string(),
        ));
    };
    let Some(name) = string_value(raw.get("name")) else {
        return Normalized::Refused(error(
            codes::FORMAT_NAME_MISSING,
            "Custom format is missing a name.",
            format!("{location}.name"),
        ));
    };
    let raw_specifications = match raw.get("specifications") {
        Some(Value::Array(items)) if !items.is_empty() => items,
        _ => {
            return Normalized::Refused(error(
                codes::FORMAT_SPECIFICATIONS_MISSING,
                "Custom format is missing its specifications array.",
                format!("{location}.specifications"),
            ));
        }
    };
    let mut diagnostics = Vec::new();
    if raw_specifications.len() > MAX_SPECIFICATIONS {
        diagnostics.push(error(
            codes::SPECIFICATIONS_TOO_MANY,
            format!("A format may contain at most {MAX_SPECIFICATIONS} specifications."),
            format!("{location}.specifications"),
        ));
    }
    let mut specifications = Vec::new();
    for (index, value) in raw_specifications
        .iter()
        .take(MAX_SPECIFICATIONS)
        .enumerate()
    {
        let spec_location = format!("{location}.specifications[{index}]");
        let Value::Object(value) = value else {
            diagnostics.push(error(
                codes::SPECIFICATION_INVALID,
                "Specification must be an object.",
                spec_location,
            ));
            continue;
        };
        let implementation = string_value(value.get("implementation"));
        let spec_name = string_value(value.get("name")).or_else(|| implementation.clone());
        let fields = normalize_fields(value.get("fields"));
        let flag_invalid = |key: &str| value.get(key).is_some_and(|flag| !flag.is_boolean());
        let (Some(implementation), Some(spec_name), Some(fields)) =
            (implementation, spec_name, fields)
        else {
            diagnostics.push(error(
                codes::SPECIFICATION_INVALID,
                "Specification needs implementation, name, and unique fields.",
                spec_location,
            ));
            continue;
        };
        if flag_invalid("negate") || flag_invalid("required") {
            diagnostics.push(error(
                codes::SPECIFICATION_INVALID,
                "Specification needs implementation, name, and unique fields.",
                spec_location,
            ));
            continue;
        }
        specifications.push(Specification {
            implementation,
            name: spec_name,
            fields,
            negate: value.get("negate") == Some(&Value::Bool(true)),
            required: value.get("required") == Some(&Value::Bool(true)),
            index,
        });
    }
    let source_id = match first_present(raw, &["trash_id", "id", "uuid", "name"]) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => js_number_to_string(number.as_f64().unwrap_or(f64::NAN)),
        _ => name.clone(),
    };
    let id = format!("{location}:{source_id}");
    let suggested_scores = normalize_scores(raw, &mut diagnostics, format!("{location}.score"));
    for diagnostic in &mut diagnostics {
        diagnostic.format_id = Some(id.clone());
    }
    Normalized::Format(ImportedFormat {
        id,
        name,
        source,
        specifications,
        suggested_scores,
        description: string_value(raw.get("description")),
        inspection_diagnostics: diagnostics,
    })
}

/// Inspects a single parsed document.
pub(super) fn inspect_document(
    document: &Value,
    source: ArrSourceApplication,
) -> Result<Inspection, ArrImportRefusal> {
    let (raw_formats, is_array) = match document {
        Value::Array(items) => (items.iter().collect::<Vec<_>>(), true),
        other => (vec![other], false),
    };
    if raw_formats.is_empty() {
        return Err(ArrImportRefusal::FormatsEmpty);
    }
    if raw_formats.len() > MAX_FORMATS {
        return Err(ArrImportRefusal::FormatsTooMany);
    }
    let mut formats = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, raw) in raw_formats.into_iter().enumerate() {
        let location = if is_array {
            format!("[{index}]")
        } else {
            "$".to_string()
        };
        match normalize_format(raw, source, &location) {
            Normalized::Format(format) => {
                diagnostics.extend(format.inspection_diagnostics.iter().cloned());
                formats.push(format);
            }
            Normalized::Refused(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    Ok(Inspection {
        formats,
        diagnostics,
    })
}

/// Size-checks, parses, and inspects pasted JSON.
pub(super) fn inspect(
    json: &str,
    source: ArrSourceApplication,
) -> Result<Inspection, ArrImportRefusal> {
    if json.len() > MAX_PASTED_JSON_BYTES {
        return Err(ArrImportRefusal::JsonTooLarge);
    }
    let document: Value = serde_json::from_str(json).map_err(|_| ArrImportRefusal::JsonInvalid)?;
    inspect_document(&document, source)
}
