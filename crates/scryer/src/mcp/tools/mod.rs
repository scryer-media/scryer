//! The tool table. No tool talks to anything outside this instance: no
//! indexer, download client or other remote service is contacted. Read and
//! dry-run tools change no server state. Write tools change saved rules and
//! acquisition configuration through the same GraphQL mutations, permission
//! checks, MFA step-up and audit trail as `/graphql`. No call deletes a
//! title, file or download, though a routing write replaces a whole scope and
//! drops the entries it leaves out. None takes or returns a credential. Each
//! tool's [`Effect`] sets the annotations `tools/list` advertises.
//!
//! Each tool declares its arguments once, nested object fields included; the
//! declaration is both the input schema `tools/list` advertises and the
//! validation `tools/call` applies, so the two cannot disagree.

mod diagnostics_overview;
mod diagnostics_section;
mod download_clients;
mod download_queue;
mod find_titles;
mod health_checks;
mod import_history;
mod indexer_errors;
mod indexers;
mod job_runs;
mod pending_releases;
pub(super) mod service_logs;
mod system_health;
mod title_decisions;
mod title_history;
mod upgrade_status;
mod whoami;

mod acquisition_reads;
mod delay_profile_write;
mod provider_types;
mod quality_profile_write;
mod routing_writes;
mod rule_sets;
mod rule_test;
mod rule_validation;
mod rule_writes;
mod title_episodes;

use std::sync::{Arc, LazyLock};

use futures_util::future::BoxFuture;
use rmcp::ErrorData;
use rmcp::model::{CallToolResult, JsonObject, Tool, ToolAnnotations};
use scryer_application::AppError;
use scryer_application::diagnostics::Sanitizer;
use serde_json::{Map, Value, json};

use super::server::tool_result;
use crate::diagnostics_routes::DiagnosticsHost;
use crate::error_reasons::{
    HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED, HTTP_DIAGNOSTICS_PERMISSION_DENIED,
    HTTP_MCP_ARGUMENTS_INVALID, HTTP_MCP_DIAGNOSTICS_FAILED,
};
use crate::middleware::{
    ApiKeyCaller, AuthState, charge_api_key_call, execute_graphql_as_api_key_caller,
};

/// Characters a free-text field keeps after sanitization.
pub(super) const FREE_TEXT_MAX_CHARS: usize = 300;

/// Characters of a result's `items` at most, measured as serialized JSON.
/// With the counts beside them a result stays near 5,000 tokens.
pub(super) const ITEMS_BUDGET_CHARS: usize = 16_000;

/// What a tool runs with: the route's auth state, the admitted caller and the
/// binary's diagnostics facts.
#[derive(Clone)]
pub(crate) struct ToolContext {
    pub auth: AuthState,
    pub caller: ApiKeyCaller,
    pub diagnostics: Arc<DiagnosticsHost>,
    /// Raised when the call ends or its client goes away; blocking work for
    /// the call checks it and stops.
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
}

pub(super) type ToolFuture = BoxFuture<'static, Result<CallToolResult, ErrorData>>;

pub(crate) struct ToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub params: &'static [Param],
    pub effect: Effect,
    pub run: fn(ToolContext, Args) -> ToolFuture,
}

/// What a call does to server state, as its annotations advertise it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Reads and changes nothing.
    Read,
    /// Computes an answer from what it is given, such as a validation or a
    /// scoring preview, and saves nothing.
    DryRun,
    /// Changes saved configuration.
    Write {
        /// Whether the call replaces or overwrites state that already exists.
        destructive: bool,
        /// Whether repeating the same call leaves the same state.
        idempotent: bool,
    },
}

/// Every tool, in the order `tools/list` returns them.
pub(crate) static TOOLS: &[&ToolSpec] = &[
    &whoami::TOOL,
    &health_checks::TOOL,
    &download_clients::TOOL,
    &indexers::TOOL,
    &indexer_errors::TOOL,
    &system_health::TOOL,
    &job_runs::TOOL,
    &find_titles::TOOL,
    &title_decisions::TOOL,
    &pending_releases::TOOL,
    &title_history::TOOL,
    &download_queue::TOOL,
    &import_history::TOOL,
    &upgrade_status::TOOL,
    &diagnostics_overview::TOOL,
    &diagnostics_section::TOOL,
    &service_logs::TOOL,
    &rule_sets::LIST_TOOL,
    &rule_sets::GET_TOOL,
    &title_episodes::TOOL,
    &rule_validation::TOOL,
    &rule_test::TOOL,
    &rule_writes::CREATE_TOOL,
    &rule_writes::UPDATE_TOOL,
    &rule_writes::TOGGLE_TOOL,
    &acquisition_reads::DELAY_PROFILES_TOOL,
    &acquisition_reads::QUALITY_PROFILES_TOOL,
    &acquisition_reads::INDEXER_ROUTING_TOOL,
    &acquisition_reads::DOWNLOAD_CLIENT_ROUTING_TOOL,
    &acquisition_reads::SEEDING_PROFILES_TOOL,
    &provider_types::INDEXER_TOOL,
    &provider_types::DOWNLOAD_CLIENT_TOOL,
    &delay_profile_write::TOOL,
    &quality_profile_write::TOOL,
    &routing_writes::INDEXER_TOOL,
    &routing_writes::DOWNLOAD_CLIENT_TOOL,
    &routing_writes::REORDER_TOOL,
    &routing_writes::MAPPING_TOOL,
];

pub(crate) fn find(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().copied().find(|tool| tool.name == name)
}

pub(crate) fn tool(spec: &ToolSpec) -> Tool {
    let annotations = ToolAnnotations::with_title(spec.title).open_world(false);
    let annotations = match spec.effect {
        Effect::Read | Effect::DryRun => annotations.read_only(true),
        Effect::Write {
            destructive,
            idempotent,
        } => annotations
            .read_only(false)
            .destructive(destructive)
            .idempotent(idempotent),
    };
    Tool::new(
        spec.name,
        spec.description,
        Arc::new(input_schema(spec.params)),
    )
    .with_title(spec.title)
    .with_annotations(annotations)
}

/// One argument a tool accepts, or one field of an object argument.
pub(crate) struct Param {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
    pub kind: Kind,
}

pub(crate) enum Kind {
    /// A boolean; without a default an omitted optional one is not sent.
    Bool { default: Option<bool> },
    Int {
        default: Option<i64>,
        min: i64,
        max: i64,
    },
    /// A string that is not blank.
    Str { max_chars: usize },
    Enum {
        values: fn() -> Vec<&'static str>,
        default: Option<&'static str>,
    },
    EnumList {
        values: fn() -> Vec<&'static str>,
        min_items: usize,
        max_items: usize,
    },
    /// Distinct non-blank strings.
    StrList {
        min_items: usize,
        max_items: usize,
        max_chars: usize,
    },
    /// Non-blank strings that may repeat, for a list the server stores as
    /// sent, so whatever a read returns can be written back.
    StrSeq {
        min_items: usize,
        max_items: usize,
        max_chars: usize,
    },
    /// An object with exactly these fields.
    Object { fields: &'static [Param] },
    /// A list of objects with exactly these fields.
    ObjectList {
        fields: &'static [Param],
        min_items: usize,
        max_items: usize,
    },
}

fn kind_schema(kind: &Kind) -> Value {
    match kind {
        Kind::Bool { default } => {
            let mut schema = json!({"type": "boolean"});
            if let Some(default) = default {
                schema["default"] = json!(default);
            }
            schema
        }
        Kind::Int { default, min, max } => {
            let mut schema = json!({"type": "integer", "minimum": min, "maximum": max});
            if let Some(default) = default {
                schema["default"] = json!(default);
            }
            schema
        }
        Kind::Str { max_chars } => {
            json!({"type": "string", "minLength": 1, "maxLength": max_chars})
        }
        Kind::Enum { values, default } => {
            let mut schema = json!({"type": "string", "enum": values()});
            if let Some(default) = default {
                schema["default"] = json!(default);
            }
            schema
        }
        Kind::EnumList {
            values,
            min_items,
            max_items,
        } => json!({
            "type": "array",
            "items": {"type": "string", "enum": values()},
            "minItems": min_items,
            "maxItems": max_items,
            "uniqueItems": true,
        }),
        Kind::StrList {
            min_items,
            max_items,
            max_chars,
        }
        | Kind::StrSeq {
            min_items,
            max_items,
            max_chars,
        } => json!({
            "type": "array",
            "items": {"type": "string", "minLength": 1, "maxLength": max_chars},
            "minItems": min_items,
            "maxItems": max_items,
            "uniqueItems": matches!(kind, Kind::StrList { .. }),
        }),
        Kind::Object { fields } => Value::Object(object_schema(fields)),
        Kind::ObjectList {
            fields,
            min_items,
            max_items,
        } => json!({
            "type": "array",
            "items": object_schema(fields),
            "minItems": min_items,
            "maxItems": max_items,
        }),
    }
}

fn object_schema(params: &[Param]) -> JsonObject {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for param in params {
        let mut schema = kind_schema(&param.kind);
        schema["description"] = json!(param.description);
        properties.insert(param.name.to_string(), schema);
        if param.required {
            required.push(param.name);
        }
    }
    let mut schema = JsonObject::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    schema.insert("additionalProperties".into(), json!(false));
    schema
}

fn input_schema(params: &[Param]) -> JsonObject {
    object_schema(params)
}

/// Arguments that passed validation, with defaults filled in.
#[derive(Debug, Default)]
pub(crate) struct Args(Map<String, Value>);

impl Args {
    pub fn bool(&self, name: &str) -> bool {
        self.0.get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        self.0.get(name).and_then(Value::as_i64)
    }

    /// An integer argument that has a default, as a count or offset.
    pub fn count(&self, name: &str) -> usize {
        self.int(name)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0)
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(Value::as_str)
    }

    pub fn list(&self, name: &str) -> Option<Vec<String>> {
        self.0.get(name).and_then(Value::as_array).map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
    }

    /// Whether the client sent the argument.
    pub fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// The validated value, for passing straight into GraphQL variables.
    pub fn value(&self, name: &str) -> Value {
        self.0.get(name).cloned().unwrap_or(Value::Null)
    }
}

/// A refused argument. `argument` is the declared path, such as
/// `entries[2].priority`: declared names and list positions only, never a
/// value or name the client sent.
pub(super) fn invalid_argument(argument: &str, message: &str) -> ErrorData {
    ErrorData::invalid_params(
        message.to_string(),
        Some(json!({"reason": HTTP_MCP_ARGUMENTS_INVALID, "argument": argument})),
    )
}

fn argument_error(path: &str, expected: &str) -> ErrorData {
    invalid_argument(path, &format!("argument `{path}` must be {expected}"))
}

/// Checks a call's arguments against the tool's declaration. Messages name
/// only the tool's own argument names, never a value or name the client sent.
pub(crate) fn validate(spec: &ToolSpec, arguments: Option<JsonObject>) -> Result<Args, ErrorData> {
    validate_object(spec.params, arguments.unwrap_or_default(), None).map(Args)
}

fn validate_object(
    params: &[Param],
    mut arguments: JsonObject,
    path: Option<&str>,
) -> Result<Map<String, Value>, ErrorData> {
    if arguments
        .keys()
        .any(|key| !params.iter().any(|param| param.name == key))
    {
        return Err(match path {
            None => ErrorData::invalid_params(
                "the arguments include a name this tool does not accept",
                Some(json!({"reason": HTTP_MCP_ARGUMENTS_INVALID})),
            ),
            Some(path) => invalid_argument(
                path,
                &format!("argument `{path}` includes a field it does not accept"),
            ),
        });
    }
    let mut checked = Map::new();
    for param in params {
        let param_path = match path {
            None => param.name.to_string(),
            Some(path) => format!("{path}.{}", param.name),
        };
        // An explicit null is the same as leaving the argument out.
        let value = arguments
            .remove(param.name)
            .filter(|value| !value.is_null());
        let value = match (&param.kind, value) {
            (_, None) if param.required => return Err(argument_error(&param_path, "given")),
            (Kind::Bool { default }, None) => default.map(|value| json!(value)),
            (Kind::Int { default, .. }, None) => default.map(|value| json!(value)),
            (Kind::Enum { default, .. }, None) => default.map(|value| json!(value)),
            (_, None) => None,
            (kind, Some(value)) => Some(validate_value(kind, value, &param_path)?),
        };
        if let Some(value) = value {
            checked.insert(param.name.to_string(), value);
        }
    }
    Ok(checked)
}

fn distinct(items: &[Value]) -> bool {
    items
        .iter()
        .enumerate()
        .all(|(index, item)| !items[..index].contains(item))
}

fn validate_value(kind: &Kind, value: Value, path: &str) -> Result<Value, ErrorData> {
    match kind {
        Kind::Bool { .. } => match value {
            Value::Bool(_) => Ok(value),
            _ => Err(argument_error(path, "a boolean")),
        },
        Kind::Int { min, max, .. } => match value.as_i64() {
            Some(number) if (*min..=*max).contains(&number) && !value.is_f64() => Ok(json!(number)),
            _ => Err(argument_error(
                path,
                &format!("an integer from {min} to {max}"),
            )),
        },
        Kind::Str { max_chars } => match value.as_str() {
            Some(text) if !text.trim().is_empty() && text.chars().count() <= *max_chars => {
                Ok(value)
            }
            _ => Err(argument_error(
                path,
                &format!("a non-blank string of at most {max_chars} characters"),
            )),
        },
        Kind::Enum { values, .. } => match value.as_str() {
            Some(text) if values().contains(&text) => Ok(value),
            _ => Err(argument_error(path, "one of the listed values")),
        },
        Kind::EnumList {
            values,
            min_items,
            max_items,
        } => {
            let allowed = values();
            let valid = value.as_array().is_some_and(|items| {
                (*min_items..=*max_items).contains(&items.len())
                    && items
                        .iter()
                        .all(|item| item.as_str().is_some_and(|text| allowed.contains(&text)))
                    && distinct(items)
            });
            if valid {
                Ok(value)
            } else {
                Err(argument_error(
                    path,
                    &format!("a list of {min_items} to {max_items} distinct listed values"),
                ))
            }
        }
        Kind::StrList {
            min_items,
            max_items,
            max_chars,
        }
        | Kind::StrSeq {
            min_items,
            max_items,
            max_chars,
        } => {
            let unique = matches!(kind, Kind::StrList { .. });
            let valid = value.as_array().is_some_and(|items| {
                (*min_items..=*max_items).contains(&items.len())
                    && items.iter().all(|item| {
                        item.as_str().is_some_and(|text| {
                            !text.trim().is_empty() && text.chars().count() <= *max_chars
                        })
                    })
                    && (!unique || distinct(items))
            });
            if valid {
                Ok(value)
            } else {
                let distinct = if unique { "distinct " } else { "" };
                Err(argument_error(
                    path,
                    &format!(
                        "a list of {min_items} to {max_items} {distinct}strings of at most {max_chars} characters"
                    ),
                ))
            }
        }
        Kind::Object { fields } => match value {
            Value::Object(object) => validate_object(fields, object, Some(path)).map(Value::Object),
            _ => Err(argument_error(path, "an object")),
        },
        Kind::ObjectList {
            fields,
            min_items,
            max_items,
        } => {
            let Value::Array(items) = value else {
                return Err(argument_error(path, "a list of objects"));
            };
            if !(*min_items..=*max_items).contains(&items.len()) {
                return Err(argument_error(
                    path,
                    &format!("a list of {min_items} to {max_items} objects"),
                ));
            }
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let item_path = format!("{path}[{index}]");
                    match item {
                        Value::Object(object) => {
                            validate_object(fields, object, Some(&item_path)).map(Value::Object)
                        }
                        _ => Err(argument_error(&item_path, "an object")),
                    }
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
    }
}

pub(super) static PATTERN_SANITIZER: LazyLock<Sanitizer> = LazyLock::new(Sanitizer::patterns_only);

/// A free-text field as a tool returns it: pattern-sanitized, then cut on a
/// character boundary.
pub(super) fn free_text(text: &str) -> String {
    let sanitized = PATTERN_SANITIZER.text(text);
    scryer_application::diagnostics::cut_chars(&sanitized, FREE_TEXT_MAX_CHARS).to_string()
}

/// [`free_text`] over a JSON field that may be null.
pub(super) fn free_text_value(value: &Value) -> Value {
    match value.as_str() {
        Some(text) => Value::String(free_text(text)),
        None => Value::Null,
    }
}

/// `object[field]`, or null.
pub(super) fn field<'v>(object: &'v Value, name: &str) -> &'v Value {
    object.get(name).unwrap_or(&Value::Null)
}

/// Copies the named fields of `source` into a new object, in that order.
pub(super) fn pick(source: &Value, names: &[&str]) -> Map<String, Value> {
    names
        .iter()
        .map(|name| (name.to_string(), field(source, name).clone()))
        .collect()
}

/// `(key, count)` pairs as an object, largest first, then by key.
pub(super) fn tally<I, K>(items: I) -> Value
where
    I: IntoIterator<Item = K>,
    K: Into<String>,
{
    let mut counts = std::collections::BTreeMap::<String, u64>::new();
    for item in items {
        *counts.entry(item.into()).or_default() += 1;
    }
    sorted_counts(counts.into_iter())
}

pub(super) fn sorted_counts(counts: impl Iterator<Item = (String, u64)>) -> Value {
    let mut counts = counts.collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    Value::Object(
        counts
            .into_iter()
            .map(|(key, count)| (key, json!(count)))
            .collect(),
    )
}

/// The serialized size of `value` in characters.
pub(super) fn json_chars(value: &Value) -> usize {
    serde_json::to_string(value).map_or(0, |text| text.chars().count())
}

/// Items that fit [`ITEMS_BUDGET_CHARS`], in order. At least one item is kept
/// so a page always makes progress.
pub(super) fn fit_budget(items: Vec<Value>) -> Vec<Value> {
    let mut used = 0;
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        let size = json_chars(&item);
        if !kept.is_empty() && used + size > ITEMS_BUDGET_CHARS {
            break;
        }
        used += size;
        kept.push(item);
    }
    kept
}

/// One offset page of `rows`: at most `limit` from `offset`, within the
/// items budget. `next_offset` is null when nothing is left.
pub(super) fn offset_page(
    rows: impl IntoIterator<Item = Value>,
    total: usize,
    offset: usize,
    limit: usize,
) -> (Vec<Value>, Value) {
    let items = fit_budget(rows.into_iter().skip(offset).take(limit).collect());
    let next = offset + items.len();
    (
        items,
        if next < total {
            json!(next)
        } else {
            Value::Null
        },
    )
}

/// Builds `{...counts, returned, items, next}` in that order.
pub(super) fn list_result(
    mut head: Map<String, Value>,
    items: Vec<Value>,
    next_key: &str,
    next: Value,
) -> CallToolResult {
    head.insert("returned".into(), json!(items.len()));
    head.insert("items".into(), Value::Array(items));
    head.insert(next_key.into(), next);
    CallToolResult::structured(Value::Object(head))
}

/// A `limit` argument: rows to return, defaulting to and capped at `max`.
pub(super) const fn limit_param(description: &'static str, max: i64) -> Param {
    Param {
        name: "limit",
        description,
        required: false,
        kind: Kind::Int {
            default: Some(max),
            min: 1,
            max,
        },
    }
}

/// An `offset` argument: rows to skip, from `next_offset`.
pub(super) const OFFSET_PARAM: Param = Param {
    name: "offset",
    description: "Rows to skip, from next_offset.",
    required: false,
    kind: Kind::Int {
        default: Some(0),
        min: 0,
        max: 10_000,
    },
};

/// The names of a GraphQL enum, for a declared argument.
pub(super) fn content_scopes() -> Vec<&'static str> {
    graphql_enum_names::<scryer_interface::types::ContentScopeValue>()
}

/// A content scope argument, required.
pub(super) const SCOPE_PARAM: Param = Param {
    name: "scope",
    description: "The content scope: MOVIE, SERIES or ANIME.",
    required: true,
    kind: Kind::Enum {
        values: content_scopes,
        default: None,
    },
};

/// Runs a mutation and returns its payload field as the tool result, or the
/// mutation's own refusal unchanged.
pub(super) async fn mutation_result(
    context: &ToolContext,
    document: &'static str,
    variables: Value,
    payload: &str,
) -> Result<CallToolResult, ErrorData> {
    let data = match graphql(context, document, variables).await {
        Ok(data) => data,
        Err(error) => return Ok(error),
    };
    let mut result = Map::new();
    result.insert(payload.into(), field(&data, payload).clone());
    Ok(CallToolResult::structured(Value::Object(result)))
}

/// Runs one GraphQL operation as the caller. `Ok` holds the data object;
/// `Err` holds the finished tool error, partial data and paths included.
pub(super) async fn graphql(
    context: &ToolContext,
    document: &'static str,
    variables: Value,
) -> Result<Value, CallToolResult> {
    let mut request = async_graphql::Request::new(document);
    if !variables.is_null() {
        request = request.variables(async_graphql::Variables::from_json(variables));
    }
    let response = execute_graphql_as_api_key_caller(&context.auth, &context.caller, request).await;
    if !response.errors.is_empty() {
        return Err(tool_result(response));
    }
    Ok(serde_json::to_value(&response.data).unwrap_or(Value::Null))
}

/// An application error as a tool error, through the same mapping GraphQL
/// uses, so its code and reason are the ones `/graphql` would send.
pub(super) fn app_error(error: AppError) -> CallToolResult {
    let error = scryer_interface::context::to_gql_error(error)
        .into_server_error(async_graphql::Pos::default());
    let mut error = error;
    error.locations.clear();
    tool_result(async_graphql::Response::from_errors(vec![error]))
}

/// A tool error built here, with a code and a reason.
pub(super) fn coded_error(code: &str, reason: &str, message: &str) -> CallToolResult {
    CallToolResult::structured_error(json!({"errors": [{
        "code": code,
        "reason": reason,
        "message": message,
    }]}))
}

/// Charges one tool call to the GraphQL rate limits. `call_tool` runs this
/// once before the tool; nothing inside a tool charges again.
pub(crate) fn charge_call(context: &ToolContext) -> Option<CallToolResult> {
    charge_api_key_call(&context.auth, &context.caller)
        .err()
        .map(tool_result)
}

/// The diagnostics tools' admission, the same as the bundle route's: a full
/// session and the system-settings permission, settled before anything is
/// read.
pub(super) async fn admit_diagnostics(context: &ToolContext) -> Option<CallToolResult> {
    if !context.caller.full_session() {
        return Some(coded_error(
            "UNAUTHORIZED",
            HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED,
            "Diagnostics need a full session.",
        ));
    }
    match context
        .auth
        .app
        .require_app_permission(
            context.caller.user(),
            scryer_domain::AppPermission::ManageSystemSettings,
        )
        .await
    {
        Ok(()) => None,
        Err(error) => Some(diagnostics_error(error)),
    }
}

/// A diagnostics read error: a permission refusal carries the bundle route's
/// reason, anything else the diagnostics failure reason.
pub(super) fn diagnostics_error(error: AppError) -> CallToolResult {
    match error {
        AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. } => coded_error(
            "UNAUTHORIZED",
            HTTP_DIAGNOSTICS_PERMISSION_DENIED,
            "Diagnostics require the system settings permission.",
        ),
        error => {
            tracing::warn!(%error, "MCP diagnostics read failed");
            coded_error(
                "INTERNAL_SERVER_ERROR",
                HTTP_MCP_DIAGNOSTICS_FAILED,
                "The diagnostics could not be read.",
            )
        }
    }
}

/// An enum's GraphQL names, for an argument that takes one.
pub(super) fn graphql_enum_names<T: async_graphql::resolver_utils::EnumType>() -> Vec<&'static str>
{
    T::items().iter().map(|item| item.name).collect()
}

/// A GraphQL enum value's name.
pub(super) fn graphql_name<T: async_graphql::InputType>(value: &T) -> Value {
    match value.to_value() {
        async_graphql::Value::Enum(name) => json!(name.as_str()),
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    }
}

#[cfg(test)]
pub(crate) fn every_operation_document() -> Vec<(&'static str, &'static str)> {
    let mut documents = vec![
        ("whoami", whoami::OPERATION),
        ("health_checks", health_checks::OPERATION),
        ("download_clients", download_clients::OPERATION),
        ("indexers", indexers::OPERATION),
        ("indexer_stats", indexers::STATS_OPERATION),
        ("indexer_errors", indexer_errors::OPERATION),
        ("system_health", system_health::OPERATION),
        ("latest_job_runs", job_runs::LATEST_OPERATION),
        ("job_runs", job_runs::OPERATION),
        ("find_titles", find_titles::OPERATION),
        ("title_decisions", title_decisions::OPERATION),
        ("pending_releases", pending_releases::OPERATION),
        ("title_history", title_history::OPERATION),
        ("import_history", import_history::OPERATION),
        ("upgrade_status", upgrade_status::OPERATION),
        ("list_rule_sets", rule_sets::LIST_OPERATION),
        ("get_rule_set", rule_sets::GET_OPERATION),
        ("title_collections", title_episodes::COLLECTIONS_OPERATION),
        ("collection_episodes", title_episodes::EPISODES_OPERATION),
        ("validate_rule_set", rule_validation::OPERATION),
        ("test_rule_set", rule_test::OPERATION),
        ("create_rule_set", rule_writes::CREATE_OPERATION),
        ("update_rule_set", rule_writes::UPDATE_OPERATION),
        ("set_rule_set_enabled", rule_writes::TOGGLE_OPERATION),
        (
            "list_delay_profiles",
            acquisition_reads::DELAY_PROFILES_OPERATION,
        ),
        (
            "get_quality_profile_settings",
            acquisition_reads::QUALITY_PROFILES_OPERATION,
        ),
        (
            "get_indexer_routing",
            acquisition_reads::INDEXER_ROUTING_OPERATION,
        ),
        (
            "get_download_client_routing",
            acquisition_reads::DOWNLOAD_CLIENT_ROUTING_OPERATION,
        ),
        (
            "list_seeding_profiles",
            acquisition_reads::SEEDING_PROFILES_OPERATION,
        ),
        (
            "list_indexer_provider_types",
            provider_types::INDEXER_OPERATION,
        ),
        (
            "list_download_client_provider_types",
            provider_types::DOWNLOAD_CLIENT_OPERATION,
        ),
        ("upsert_delay_profile", delay_profile_write::OPERATION),
        (
            "save_quality_profile_settings",
            quality_profile_write::OPERATION,
        ),
        ("update_indexer_routing", routing_writes::INDEXER_OPERATION),
        (
            "update_download_client_routing",
            routing_writes::DOWNLOAD_CLIENT_OPERATION,
        ),
        (
            "reorder_download_clients",
            routing_writes::REORDER_OPERATION,
        ),
        (
            "set_indexer_download_client_mapping",
            routing_writes::MAPPING_OPERATION,
        ),
    ];
    documents.sort_by_key(|(name, _)| *name);
    documents
}
