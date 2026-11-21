//! `list_operations`: the catalog as compact rows, grouped by skill.

use super::{
    Arguments, ToolContext, ToolEntry, ToolFuture, api_class, coded_error_with, local_read, tool,
};
use crate::error_reasons::HTTP_MCP_SKILL_UNKNOWN;
use crate::mcp::catalog::{CATALOG, Operation, one_line};
use crate::mcp::schema_index::{SchemaIndex, schema_index};
use crate::mcp::server::json_result;
use serde_json::{Map, Value, json};

pub(super) fn entry() -> ToolEntry {
    ToolEntry {
        tool: tool(
            "list_operations",
            "Operations run_query and run_mutation accept, by skill, as [name, kind q|m, purpose] rows.",
            json!({"type": "object", "properties": {"skill": {"type": "string"}}}),
            local_read(),
        ),
        accepts: &["skill"],
        run,
        class: api_class,
    }
}

fn run(context: ToolContext, arguments: Arguments) -> ToolFuture {
    Box::pin(async move {
        let skill = arguments.str("skill", 100)?;
        let skills = CATALOG.skills();
        if let Some(skill) = skill
            && !skills.contains(&skill)
        {
            let mut facts = Map::new();
            facts.insert("skills".into(), json!(skills));
            return Ok(coded_error_with(
                "BAD_USER_INPUT",
                HTTP_MCP_SKILL_UNKNOWN,
                "No served operation is in this skill.",
                facts,
            ));
        }
        // The schema is read only for an operation with no comment of its own.
        let wanted =
            |operation: &Operation| skill.is_none_or(|skill| operation.skills.contains(&skill));
        let index = if CATALOG
            .operations()
            .iter()
            .any(|operation| wanted(operation) && operation.purpose.is_none())
        {
            schema_index(&context.auth).await
        } else {
            None
        };
        let mut groups = Map::new();
        for name in skills
            .iter()
            .filter(|name| skill.is_none_or(|skill| skill == **name))
        {
            let rows = CATALOG
                .operations()
                .iter()
                .filter(|operation| operation.skills.contains(name))
                .map(|operation| row(operation, index.as_deref()))
                .collect::<Vec<_>>();
            groups.insert((*name).to_string(), Value::Array(rows));
        }
        Ok(json_result(Value::Object(groups)))
    })
}

fn row(operation: &Operation, index: Option<&SchemaIndex>) -> Value {
    let purpose = operation.purpose.clone().or_else(|| {
        let index = index?;
        let mutation = operation.kind == crate::mcp::catalog::OperationKind::Mutation;
        let described = operation
            .root_fields
            .iter()
            .filter_map(|field| index.root_field(mutation, field))
            .collect::<Vec<_>>()
            .join(" ");
        one_line(&described)
    });
    match purpose {
        Some(purpose) => json!([operation.name, operation.kind.token(), purpose]),
        None => json!([operation.name, operation.kind.token()]),
    }
}
