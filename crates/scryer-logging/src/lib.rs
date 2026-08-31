//! Structured logging primitives shared by the Scryer runtime.
//!
//! The public context types deliberately contain operational identifiers only.
//! Callers must never place credentials, request bodies, GraphQL documents, or
//! session material in a [`LogContext`].

use std::fmt;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

const CONTEXT_FIELD: &str = "scryer_context";
const REDACTED_VALUE: &str = "[REDACTED]";

/// Dedicated tracing target kept enabled so canonical context survives
/// restrictive application log filters without emitting an extra log record.
pub const CONTEXT_TARGET: &str = "scryer_context";

/// Ensures canonical context spans stay enabled regardless of the configured
/// application event filter.
pub fn enable_context_spans(
    filter: tracing_subscriber::EnvFilter,
) -> tracing_subscriber::EnvFilter {
    filter.add_directive(
        format!("{CONTEXT_TARGET}=trace")
            .parse()
            .expect("context tracing directive must be valid"),
    )
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct LogContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<ActorContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<WorkflowContext>,
    #[serde(default, skip_serializing_if = "ResourceContext::is_empty")]
    pub resource: ResourceContext,
}

impl LogContext {
    pub fn request(request: RequestContext) -> Self {
        Self {
            request: Some(request),
            ..Self::default()
        }
    }

    pub fn workflow(workflow: WorkflowContext) -> Self {
        Self {
            workflow: Some(workflow),
            ..Self::default()
        }
    }

    pub fn with_actor(mut self, actor: ActorContext) -> Self {
        self.actor = Some(actor);
        self
    }

    pub fn with_resource(mut self, resource: ResourceContext) -> Self {
        self.resource = resource;
        self
    }

    pub fn merge_from(&mut self, child: &Self) {
        if child.request.is_some() {
            self.request = child.request.clone();
        }
        if child.actor.is_some() {
            self.actor = child.actor.clone();
        }
        if child.workflow.is_some() {
            self.workflow = child.workflow.clone();
        }
        self.resource.merge_from(&child.resource);
    }

    fn is_empty(&self) -> bool {
        self.request.is_none()
            && self.actor.is_none()
            && self.workflow.is_none()
            && self.resource.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RequestContext {
    pub id: String,
    pub transport: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ActorContext {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct WorkflowContext {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ResourceContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexer_id: Option<String>,
}

impl ResourceContext {
    fn is_empty(&self) -> bool {
        self.title_id.is_none()
            && self.import_id.is_none()
            && self.download_id.is_none()
            && self.job_id.is_none()
            && self.client_id.is_none()
            && self.indexer_id.is_none()
    }

    fn merge_from(&mut self, child: &Self) {
        merge_option(&mut self.title_id, &child.title_id);
        merge_option(&mut self.import_id, &child.import_id);
        merge_option(&mut self.download_id, &child.download_id);
        merge_option(&mut self.job_id, &child.job_id);
        merge_option(&mut self.client_id, &child.client_id);
        merge_option(&mut self.indexer_id, &child.indexer_id);
    }
}

fn merge_option<T: Clone>(target: &mut Option<T>, child: &Option<T>) {
    if child.is_some() {
        *target = child.clone();
    }
}

/// Creates a span that supplies canonical context to all nested tracing events.
pub fn context_span(context: LogContext) -> tracing::Span {
    let encoded = serde_json::to_string(&context).expect("log context must serialize");
    tracing::span!(
        target: CONTEXT_TARGET,
        Level::TRACE,
        "scryer.context",
        scryer_context = encoded.as_str()
    )
}

/// Replaces the canonical context attached to a span created by [`context_span`].
///
/// This supports long-lived connections whose authenticated actor is only known
/// after the connection has been accepted.
pub fn update_context(span: &tracing::Span, context: LogContext) {
    let encoded = serde_json::to_string(&context).expect("log context must serialize");
    span.record(CONTEXT_FIELD, encoded.as_str());
}

#[derive(Clone, Debug)]
struct StoredLogContext(LogContext);

/// Captures [`LogContext`] values from context spans for [`JsonContextFormatter`].
#[derive(Default)]
pub struct LogContextLayer;

impl<S> Layer<S> for LogContextLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        ctx: Context<'_, S>,
    ) {
        let mut visitor = ContextFieldVisitor::default();
        attrs.record(&mut visitor);
        let Some(encoded) = visitor.encoded else {
            return;
        };
        let Ok(context) = serde_json::from_str::<LogContext>(&encoded) else {
            return;
        };
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(StoredLogContext(context));
        }
    }

    fn on_record(&self, id: &tracing::Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        let mut visitor = ContextFieldVisitor::default();
        values.record(&mut visitor);
        let Some(encoded) = visitor.encoded else {
            return;
        };
        let Ok(context) = serde_json::from_str::<LogContext>(&encoded) else {
            return;
        };
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().replace(StoredLogContext(context));
        }
    }
}

#[derive(Default)]
struct ContextFieldVisitor {
    encoded: Option<String>,
}

impl Visit for ContextFieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == CONTEXT_FIELD {
            self.encoded = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() != CONTEXT_FIELD {
            return;
        }
        let rendered = format!("{value:?}");
        self.encoded = serde_json::from_str(&rendered).ok().or(Some(rendered));
    }
}

/// Formats newline-delimited JSON events with a stable root `context` object.
#[derive(Default)]
pub struct JsonContextFormatter;

impl<S, N> FormatEvent<S, N> for JsonContextFormatter
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let mut fields = JsonFieldVisitor::default();
        event.record(&mut fields);

        let mut root = Map::new();
        root.insert(
            "timestamp".to_owned(),
            Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        );
        root.insert(
            "level".to_owned(),
            Value::String(metadata.level().to_string()),
        );
        root.insert(
            "target".to_owned(),
            Value::String(metadata.target().to_owned()),
        );
        root.insert("fields".to_owned(), Value::Object(fields.fields));

        let mut context = LogContext::default();
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(stored) = extensions.get::<StoredLogContext>() {
                    context.merge_from(&stored.0);
                }
            }
        }
        if !context.is_empty() {
            root.insert(
                "context".to_owned(),
                serde_json::to_value(context).map_err(|_| fmt::Error)?,
            );
        }

        let rendered = serde_json::to_string(&Value::Object(root)).map_err(|_| fmt::Error)?;
        writer.write_str(&rendered)?;
        writer.write_char('\n')
    }
}

#[derive(Default)]
struct JsonFieldVisitor {
    fields: Map<String, Value>,
}

impl JsonFieldVisitor {
    fn insert(&mut self, field: &Field, value: Value) {
        let value = if is_sensitive_field_name(field.name()) {
            Value::String(REDACTED_VALUE.to_owned())
        } else {
            value
        };
        self.fields.insert(field.name().to_owned(), value);
    }
}

fn is_sensitive_field_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    let compact = normalized
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    normalized
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(is_sensitive_field_segment)
        || matches!(compact.as_str(), "apikey" | "apikeys" | "bearertoken")
}

fn is_sensitive_field_segment(segment: &str) -> bool {
    matches!(
        segment,
        "authorization"
            | "cookie"
            | "cookies"
            | "credential"
            | "credentials"
            | "password"
            | "passwords"
            | "secret"
            | "secrets"
            | "token"
            | "tokens"
            | "session"
            | "sessions"
            | "headers"
            | "header"
            | "request"
            | "body"
            | "payload"
            | "query"
            | "variables"
            | "variable"
            | "document"
            | "graphql"
            | "apikey"
    )
}

impl Visit for JsonFieldVisitor {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.insert(field, Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.insert(field, Value::String(format!("{value:?}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    struct SharedWriteHandle(Arc<Mutex<Vec<u8>>>);

    impl io::Write for SharedWriteHandle {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("lock writer").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = SharedWriteHandle;

        fn make_writer(&'writer self) -> Self::Writer {
            SharedWriteHandle(self.0.clone())
        }
    }

    #[test]
    fn child_context_overrides_matching_values_and_retains_parent_resources() {
        let mut parent = LogContext::request(RequestContext {
            id: "request-1".to_owned(),
            transport: "graphql_http".to_owned(),
            operation_name: None,
            operation_type: None,
            client_ip: None,
        })
        .with_resource(ResourceContext {
            title_id: Some("title-1".to_owned()),
            ..ResourceContext::default()
        });
        let child = LogContext::workflow(WorkflowContext {
            kind: "import".to_owned(),
            id: "import-1".to_owned(),
        })
        .with_resource(ResourceContext {
            import_id: Some("import-1".to_owned()),
            ..ResourceContext::default()
        });

        parent.merge_from(&child);

        assert_eq!(
            parent.request.as_ref().map(|request| request.id.as_str()),
            Some("request-1")
        );
        assert_eq!(
            parent
                .workflow
                .as_ref()
                .map(|workflow| workflow.kind.as_str()),
            Some("import")
        );
        assert_eq!(parent.resource.title_id.as_deref(), Some("title-1"));
        assert_eq!(parent.resource.import_id.as_deref(), Some("import-1"));
    }

    #[test]
    fn serialization_omits_absent_context_values() {
        let value = serde_json::to_value(LogContext::default()).expect("serialize context");
        assert_eq!(value, serde_json::json!({}));
    }

    #[test]
    fn formatter_emits_one_json_record_with_inherited_context() {
        let output = SharedWriter::default();
        let subscriber = tracing_subscriber::registry().with(LogContextLayer).with(
            tracing_subscriber::fmt::layer()
                .event_format(JsonContextFormatter)
                .with_writer(output.clone())
                .with_ansi(false),
        );

        tracing::subscriber::with_default(subscriber, || {
            let span = context_span(
                LogContext::request(RequestContext {
                    id: "request-1".to_owned(),
                    transport: "graphql_http".to_owned(),
                    operation_name: Some("SystemHealth".to_owned()),
                    operation_type: Some("query".to_owned()),
                    client_ip: Some("127.0.0.1".to_owned()),
                })
                .with_actor(ActorContext {
                    kind: "user".to_owned(),
                    id: Some("user-1".to_owned()),
                    display_name: Some("Sam".to_owned()),
                    source: Some("authenticated_token".to_owned()),
                }),
            );
            let _guard = span.enter();
            tracing::info!(count = 2_u64, "contextual event");
        });

        let raw =
            String::from_utf8(output.0.lock().expect("lock output").clone()).expect("utf8 output");
        assert_eq!(raw.lines().count(), 1);
        let event: Value = serde_json::from_str(raw.trim()).expect("valid JSON event");
        assert_eq!(event["level"], "INFO");
        assert_eq!(event["fields"]["count"], 2);
        assert_eq!(event["context"]["request"]["id"], "request-1", "{raw}");
        assert_eq!(event["context"]["actor"]["display_name"], "Sam");
        assert!(event.to_string().contains("contextual event"));
    }

    #[test]
    fn updated_span_context_is_used_by_subsequent_events() {
        let output = SharedWriter::default();
        let subscriber = tracing_subscriber::registry().with(LogContextLayer).with(
            tracing_subscriber::fmt::layer()
                .event_format(JsonContextFormatter)
                .with_writer(output.clone())
                .with_ansi(false),
        );
        let request = RequestContext {
            id: "connection-1".to_owned(),
            transport: "graphql_ws".to_owned(),
            operation_name: None,
            operation_type: None,
            client_ip: Some("127.0.0.1".to_owned()),
        };

        tracing::subscriber::with_default(subscriber, || {
            let span = context_span(LogContext::request(request.clone()));
            update_context(
                &span,
                LogContext::request(request).with_actor(ActorContext {
                    kind: "user".to_owned(),
                    id: Some("user-1".to_owned()),
                    display_name: Some("Sam".to_owned()),
                    source: Some("authenticated_token".to_owned()),
                }),
            );
            let _guard = span.enter();
            tracing::info!("authenticated connection event");
        });

        let raw =
            String::from_utf8(output.0.lock().expect("lock output").clone()).expect("utf8 output");
        let event: Value = serde_json::from_str(raw.trim()).expect("valid JSON event");
        assert_eq!(event["context"]["request"]["id"], "connection-1");
        assert_eq!(event["context"]["actor"]["display_name"], "Sam");
    }

    #[test]
    fn formatter_redacts_sensitive_event_fields() {
        let output = SharedWriter::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .event_format(JsonContextFormatter)
                .with_writer(output.clone())
                .with_ansi(false),
        );

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                authorization = "Bearer secret-token",
                api_key = "secret-api-key",
                cookie = "session=secret",
                graphql_document = "mutation ResetPassword { resetPassword }",
                query = "query Dangerous($password: String!) { resetPassword }",
                variables = "{\"password\":\"secret\"}",
                safe_field = "kept",
                "sensitive fields must not reach JSON logs"
            );
        });

        let raw =
            String::from_utf8(output.0.lock().expect("lock output").clone()).expect("utf8 output");
        let event: Value = serde_json::from_str(raw.trim()).expect("valid JSON event");
        assert_eq!(event["fields"]["authorization"], REDACTED_VALUE);
        assert_eq!(event["fields"]["api_key"], REDACTED_VALUE);
        assert_eq!(event["fields"]["cookie"], REDACTED_VALUE);
        assert_eq!(event["fields"]["graphql_document"], REDACTED_VALUE);
        assert_eq!(event["fields"]["query"], REDACTED_VALUE);
        assert_eq!(event["fields"]["variables"], REDACTED_VALUE);
        assert_eq!(event["fields"]["safe_field"], "kept");
        assert!(!raw.contains("secret-token"));
        assert!(!raw.contains("ResetPassword"));
    }

    #[test]
    fn context_survives_a_warn_only_event_filter_when_context_target_is_disabled_in_env() {
        let output = SharedWriter::default();
        let subscriber = tracing_subscriber::registry()
            .with(enable_context_spans(tracing_subscriber::EnvFilter::new(
                "warn,scryer_context=off",
            )))
            .with(LogContextLayer)
            .with(
                tracing_subscriber::fmt::layer()
                    .event_format(JsonContextFormatter)
                    .with_writer(output.clone())
                    .with_ansi(false),
            );

        tracing::subscriber::with_default(subscriber, || {
            let span = context_span(LogContext::request(RequestContext {
                id: "request-1".to_owned(),
                transport: "graphql_http".to_owned(),
                operation_name: None,
                operation_type: None,
                client_ip: None,
            }));
            let _guard = span.enter();
            tracing::warn!("warn event retains context");
        });

        let raw =
            String::from_utf8(output.0.lock().expect("lock output").clone()).expect("utf8 output");
        let event: Value = serde_json::from_str(raw.trim()).expect("valid JSON event");
        assert_eq!(event["context"]["request"]["id"], "request-1");
    }
}
