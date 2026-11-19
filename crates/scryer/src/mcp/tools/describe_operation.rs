//! `describe_operation`: what a caller needs to build an operation's
//! variables, read from the catalog and the served schema.

use std::collections::{BTreeSet, VecDeque};

use super::{
    Arguments, ToolContext, ToolEntry, ToolFuture, api_class, coded_error, coded_error_with,
    local_read, tool,
};
use crate::error_reasons::{
    HTTP_MCP_OPERATION_AMBIGUOUS, HTTP_MCP_OPERATION_UNKNOWN, HTTP_MCP_SCHEMA_UNREADABLE,
};
use crate::mcp::catalog::{CATALOG, Operation, Resolution, cut_line};
use crate::mcp::schema_index::{NamedType, SchemaIndex, schema_index};
use crate::mcp::server::json_result;
use async_graphql::parser::types::Type;
use rmcp::ErrorData;
use rmcp::model::CallToolResult;
use serde_json::{Map, Value, json};

/// Input-object levels expanded below the variables.
const MAX_TYPE_DEPTH: usize = 4;
/// Types one description expands at most.
const MAX_TYPES: usize = 30;
/// Characters of one field, value or type description.
const DESCRIPTION_MAX_CHARS: usize = 200;
/// Serialized bytes of one description's expanded types, half the run cap
/// so the notes, variables and document fit beside them.
const TYPES_MAX_BYTES: usize = super::RESULT_MAX_BYTES / 2;
/// Characters of an operation name.
pub(super) const NAME_MAX_CHARS: usize = 100;

const BUILT_IN_SCALARS: &[&str] = &["String", "Int", "Float", "Boolean", "ID"];

pub(super) fn entry() -> ToolEntry {
    ToolEntry {
        tool: tool(
            "describe_operation",
            "An operation's notes, variables and the input and enum types they use; document: true adds its GraphQL text.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "skill": skill_property(),
                    "document": {"type": "boolean"},
                },
                "required": ["name"],
            }),
            local_read(),
        ),
        accepts: &["name", "skill", "document"],
        run,
        class: api_class,
    }
}

/// The optional `skill` argument the operation tools share. A plain string:
/// a skill that does not carry the name is refused as an unknown operation.
pub(super) fn skill_property() -> Value {
    json!({"type": "string"})
}

/// The operation `name` picks, within the `skill` argument when given;
/// otherwise the tool result that says why none is picked.
pub(super) fn find_operation(
    arguments: &Arguments,
    name: &str,
) -> Result<Result<&'static Operation, CallToolResult>, ErrorData> {
    let skill = arguments.str("skill", NAME_MAX_CHARS)?;
    Ok(match CATALOG.resolve(name, skill) {
        Resolution::Found(operation) => Ok(operation),
        Resolution::Unknown(skills) => {
            let mut facts = Map::new();
            if !skills.is_empty() {
                facts.insert("skills".into(), json!(skills));
            }
            Err(coded_error_with(
                "BAD_USER_INPUT",
                HTTP_MCP_OPERATION_UNKNOWN,
                "No catalog operation has this name here; see list_operations.",
                facts,
            ))
        }
        Resolution::Ambiguous(skills) => {
            let mut facts = Map::new();
            facts.insert("skills".into(), json!(skills));
            Err(coded_error_with(
                "BAD_USER_INPUT",
                HTTP_MCP_OPERATION_AMBIGUOUS,
                "This operation differs between skills; name one with skill.",
                facts,
            ))
        }
    })
}

fn run(context: ToolContext, arguments: Arguments) -> ToolFuture {
    Box::pin(async move {
        let name = arguments.required_str("name", NAME_MAX_CHARS)?;
        let with_document = arguments.bool("document")?.unwrap_or(false);
        let operation = match find_operation(&arguments, name)? {
            Ok(operation) => operation,
            Err(refused) => return Ok(refused),
        };
        let Some(index) = schema_index(&context.auth).await else {
            return Ok(coded_error(
                "INTERNAL_SERVER_ERROR",
                HTTP_MCP_SCHEMA_UNREADABLE,
                "The schema could not be read.",
            ));
        };
        let mut result = Map::new();
        result.insert("name".into(), json!(operation.name));
        result.insert("kind".into(), json!(operation.kind.token()));
        if !operation.notes.is_empty() {
            result.insert("notes".into(), json!(operation.notes.join("\n")));
        }
        let mut variables = Map::new();
        let mut roots = Vec::new();
        for variable in &operation.variables {
            let value = match &variable.default {
                None => json!(variable.ty),
                Some(default) => json!({"type": variable.ty, "default": default}),
            };
            variables.insert(variable.name.clone(), value);
            if let Some(ty) = Type::new(&variable.ty) {
                roots.push(crate::mcp::schema_index::base_name(&ty).to_string());
            }
        }
        if !variables.is_empty() {
            result.insert("variables".into(), Value::Object(variables));
        }
        let (types, unexpanded) = reachable_types(&index, roots, TYPES_MAX_BYTES);
        if !types.is_empty() {
            result.insert("types".into(), Value::Object(types));
        }
        if !unexpanded.is_empty() {
            result.insert("unexpanded".into(), json!(unexpanded));
        }
        if with_document {
            result.insert("document".into(), json!(operation.text));
        }
        Ok(json_result(Value::Object(result)))
    })
}

/// Every input object, enum and described scalar reachable from `roots`,
/// each once, breadth first. A type that would pass [`MAX_TYPES`] or take
/// the serialized types past `max_bytes`, and an input object deeper than
/// [`MAX_TYPE_DEPTH`], is named in the second list instead, and the types
/// it reaches are not followed.
fn reachable_types(
    index: &SchemaIndex,
    roots: Vec<String>,
    max_bytes: usize,
) -> (Map<String, Value>, Vec<String>) {
    let mut types = Map::new();
    let mut bytes = 2;
    let mut unexpanded = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut queue = roots
        .into_iter()
        .map(|name| (name, 0))
        .collect::<VecDeque<_>>();
    while let Some((name, depth)) = queue.pop_front() {
        if BUILT_IN_SCALARS.contains(&name.as_str()) || !seen.insert(name.clone()) {
            continue;
        }
        let Some(named) = index.named(&name) else {
            continue;
        };
        if types.len() >= MAX_TYPES
            || (depth > MAX_TYPE_DEPTH && matches!(named, NamedType::Input { .. }))
        {
            unexpanded.insert(name);
            continue;
        }
        let mut entry = Map::new();
        let mut reached = Vec::new();
        match named {
            NamedType::Input {
                description,
                fields,
            } => {
                insert_description(&mut entry, description.as_deref());
                let mut shown = Map::new();
                for field in fields {
                    let description = field
                        .description
                        .as_deref()
                        .and_then(|text| cut_line(text, DESCRIPTION_MAX_CHARS));
                    let value = if description.is_none() && field.default.is_none() {
                        json!(field.ty)
                    } else {
                        let mut detail = Map::new();
                        detail.insert("type".into(), json!(field.ty));
                        if let Some(description) = description {
                            detail.insert("description".into(), json!(description));
                        }
                        if let Some(default) = &field.default {
                            detail.insert("default".into(), json!(default));
                        }
                        Value::Object(detail)
                    };
                    shown.insert(field.name.clone(), value);
                    reached.push(field.base.clone());
                }
                entry.insert("fields".into(), Value::Object(shown));
            }
            NamedType::Enum {
                description,
                values,
            } => {
                insert_description(&mut entry, description.as_deref());
                entry.insert(
                    "enum".into(),
                    json!(values.iter().map(|(value, _)| value).collect::<Vec<_>>()),
                );
                let notes = values
                    .iter()
                    .filter_map(|(value, description)| {
                        let description = cut_line(description.as_deref()?, DESCRIPTION_MAX_CHARS)?;
                        Some((value.clone(), json!(description)))
                    })
                    .collect::<Map<_, _>>();
                if !notes.is_empty() {
                    entry.insert("notes".into(), Value::Object(notes));
                }
            }
            NamedType::Scalar { description } => {
                if let Some(description) = cut_line(description, DESCRIPTION_MAX_CHARS) {
                    entry.insert("scalar".into(), json!(description));
                }
            }
        }
        let entry = Value::Object(entry);
        // `"name":entry`, and a comma after an earlier one.
        let entry_bytes = json!(name).to_string().len()
            + 1
            + entry.to_string().len()
            + usize::from(!types.is_empty());
        if bytes + entry_bytes > max_bytes {
            unexpanded.insert(name);
            continue;
        }
        bytes += entry_bytes;
        types.insert(name, entry);
        queue.extend(reached.into_iter().map(|base| (base, depth + 1)));
    }
    (types, unexpanded.into_iter().collect())
}

fn insert_description(entry: &mut Map<String, Value>, description: Option<&str>) {
    if let Some(description) = description.and_then(|text| cut_line(text, DESCRIPTION_MAX_CHARS)) {
        entry.insert("description".into(), json!(description));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_past_the_byte_budget_are_named_unexpanded_and_not_followed() {
        let index = SchemaIndex::from_sdl(
            "type Query { fixture(input: OuterInput!): String }\n\
input OuterInput { \"A long note about the inner input.\" inner: InnerInput\n  kind: FixtureKind }\n\
input InnerInput { deepest: DeepestInput }\n\
input DeepestInput { value: String }\n\
enum FixtureKind { ALPHA BETA }\n",
        )
        .expect("index");
        let roots = vec!["OuterInput".to_string()];
        let (all, none) = reachable_types(&index, roots.clone(), usize::MAX);
        assert_eq!(all.len(), 4);
        assert!(none.is_empty());

        let outer_bytes = Value::Object(
            all.iter()
                .filter(|(name, _)| *name == "OuterInput")
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        )
        .to_string()
        .len();
        // Room for the outer input only: what it reaches is named, and the
        // input past that one is never reached.
        let (types, unexpanded) = reachable_types(&index, roots.clone(), outer_bytes);
        assert_eq!(types.keys().collect::<Vec<_>>(), ["OuterInput"]);
        assert!(Value::Object(types).to_string().len() <= outer_bytes);
        assert_eq!(unexpanded, ["FixtureKind", "InnerInput"]);

        let (types, unexpanded) = reachable_types(&index, roots, 1);
        assert!(types.is_empty());
        assert_eq!(unexpanded, ["OuterInput"]);
    }
}
