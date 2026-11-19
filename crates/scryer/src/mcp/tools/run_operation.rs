//! `run_query` and `run_mutation`: one catalog operation, run through
//! GraphQL as the key's account with the caller's variables unchanged.

use super::describe_operation::{NAME_MAX_CHARS, find_operation, skill_property};
use super::{
    Arguments, ToolContext, ToolEntry, ToolFuture, coded_error_with, error_entry, execute,
    invalid_argument, tool,
};
use crate::error_reasons::{HTTP_MCP_OPERATION_KIND_MISMATCH, HTTP_MCP_RESULT_TOO_LARGE};
use crate::mcp::catalog::{CATALOG, OperationKind, Resolution};
use crate::mcp::server::{graphql_value, json_error, text_result};
use crate::rate_limit::GraphqlRateLimitClass;
use rmcp::model::{CallToolResult, ToolAnnotations};
use serde_json::{Map, Value, json};

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "skill": skill_property(),
            "variables": {"type": "object"},
        },
        "required": ["name"],
    })
}

pub(super) fn query_entry() -> ToolEntry {
    ToolEntry {
        // Some queries ask an indexer, download client or other service.
        tool: tool(
            "run_query",
            "Runs a catalog query with its variables as this account.",
            schema(),
            ToolAnnotations::new().read_only(true).open_world(true),
        ),
        accepts: &["name", "skill", "variables"],
        run: |context, arguments| run(context, arguments, OperationKind::Query),
        class: |arguments| class(arguments, OperationKind::Query),
    }
}

pub(super) fn mutation_entry() -> ToolEntry {
    ToolEntry {
        tool: tool(
            "run_mutation",
            "Runs a catalog mutation with its variables as this account.",
            schema(),
            ToolAnnotations::new()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        ),
        accepts: &["name", "skill", "variables"],
        run: |context, arguments| run(context, arguments, OperationKind::Mutation),
        class: |arguments| class(arguments, OperationKind::Mutation),
    }
}

/// The class `/graphql` charges the named operation to, when the arguments
/// name one operation of this tool's kind; one API request otherwise, as
/// for any refusal.
fn class(arguments: &Arguments, expected: OperationKind) -> GraphqlRateLimitClass {
    let Ok(Some(name)) = arguments.str("name", NAME_MAX_CHARS) else {
        return GraphqlRateLimitClass::Api;
    };
    let Ok(skill) = arguments.str("skill", NAME_MAX_CHARS) else {
        return GraphqlRateLimitClass::Api;
    };
    match CATALOG.resolve(name, skill) {
        Resolution::Found(operation) if operation.kind == expected => operation.class,
        _ => GraphqlRateLimitClass::Api,
    }
}

fn run(context: ToolContext, mut arguments: Arguments, expected: OperationKind) -> ToolFuture {
    Box::pin(async move {
        let name = arguments.required_str("name", NAME_MAX_CHARS)?.to_string();
        let variables = match arguments.take("variables") {
            None => None,
            Some(Value::Object(variables)) => Some(Value::Object(variables)),
            Some(_) => return Err(invalid_argument("variables", "must be an object")),
        };
        let operation = match find_operation(&arguments, &name)? {
            Ok(operation) => operation,
            Err(refused) => return Ok(refused),
        };
        if operation.kind != expected {
            let mut facts = Map::new();
            facts.insert("tool".into(), json!(operation.kind.tool()));
            return Ok(coded_error_with(
                "BAD_USER_INPUT",
                HTTP_MCP_OPERATION_KIND_MISMATCH,
                "This operation runs through the other run tool.",
                facts,
            ));
        }
        let mut request =
            async_graphql::Request::new(operation.text).operation_name(operation.name.as_str());
        if let Some(variables) = variables {
            request = request.variables(async_graphql::Variables::from_json(variables));
        }
        let (value, failed) = graphql_value(execute(&context, request).await);
        Ok(capped(value, failed, context.result_max_bytes, expected))
    })
}

/// The result of `value` when its serialized text fits `max_bytes`.
/// Otherwise a coded error with both sizes, never a cut result: a success
/// becomes that error alone; an error keeps its errors, drops its partial
/// data and gains that error. For a mutation whose data has any non-null
/// root field, failed or not, the error says `applied`: something ran.
fn capped(value: Value, failed: bool, max_bytes: usize, kind: OperationKind) -> CallToolResult {
    let text = value.to_string();
    if text.len() <= max_bytes {
        return text_result(text, failed);
    }
    let mut facts = Map::new();
    facts.insert("result_bytes".into(), json!(text.len()));
    facts.insert("max_bytes".into(), json!(max_bytes));
    if kind == OperationKind::Mutation && any_root_field_set(&value, failed) {
        facts.insert("applied".into(), json!(true));
    }
    let message = "The result is too large; narrow it with paging or filter variables.";
    let too_large = error_entry(
        "PAYLOAD_TOO_LARGE",
        HTTP_MCP_RESULT_TOO_LARGE,
        message,
        facts,
    );
    let mut errors = match (failed, value) {
        (true, Value::Object(mut content)) => match content.remove("errors") {
            Some(Value::Array(errors)) => errors,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    errors.push(too_large);
    json_error(json!({"errors": errors}))
}

/// Whether the response data has a non-null root field. A success is the
/// data itself; a failure carries it under `data`.
fn any_root_field_set(value: &Value, failed: bool) -> bool {
    let data = if failed {
        value.get("data")
    } else {
        Some(value)
    };
    data.and_then(Value::as_object)
        .is_some_and(|fields| fields.values().any(|field| !field.is_null()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A result's one text block, parsed.
    fn parsed(result: &CallToolResult) -> Value {
        assert!(result.structured_content.is_none());
        assert_eq!(result.content.len(), 1);
        let text = result.content[0].as_text().expect("a text block");
        serde_json::from_str(&text.text).expect("JSON text")
    }

    #[test]
    fn an_oversized_result_is_refused_with_its_sizes_never_cut() {
        let big = json!({"items": "x".repeat(200)});
        let query = capped(big.clone(), false, 100, OperationKind::Query);
        assert_eq!(query.is_error, Some(true));
        let content = parsed(&query);
        let error = &content["errors"][0];
        assert_eq!(error["reason"], HTTP_MCP_RESULT_TOO_LARGE);
        assert_eq!(error["max_bytes"], 100);
        assert!(error["result_bytes"].as_u64().unwrap() > 200);
        assert!(error.get("applied").is_none());

        let mutation = capped(big.clone(), false, 100, OperationKind::Mutation);
        assert_eq!(parsed(&mutation)["errors"][0]["applied"], true);

        // A mutation that partly failed but set a root field still ran.
        let partial = capped(
            json!({
                "errors": [{"code": "FIXTURE", "message": "fixture"}],
                "data": {"first": big, "second": null},
            }),
            true,
            100,
            OperationKind::Mutation,
        );
        let content = parsed(&partial);
        assert!(content.get("data").is_none(), "{content}");
        assert_eq!(content["errors"][0]["code"], "FIXTURE");
        assert_eq!(content["errors"][1]["reason"], HTTP_MCP_RESULT_TOO_LARGE);
        assert_eq!(content["errors"][1]["applied"], true);

        // A failure whose root fields are all null applied nothing.
        let none_set = capped(
            json!({
                "errors": [{"code": "FIXTURE", "message": "x".repeat(200)}],
                "data": {"first": null},
            }),
            true,
            100,
            OperationKind::Mutation,
        );
        assert!(parsed(&none_set)["errors"][1].get("applied").is_none());

        let small = capped(json!({"a": 1}), false, 100, OperationKind::Query);
        assert_eq!(small.is_error, Some(false));
        assert_eq!(parsed(&small), json!({"a": 1}));
    }
}
