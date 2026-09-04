//! GraphQL execution metrics as an `async-graphql` extension.
//!
//! Scryer serves nearly all of its API through a single `/graphql` route, so HTTP metrics alone
//! cannot tell an operator *which* part of the API is slow or erroring. This extension adds the
//! per-operation view: one count, one latency sample and an error tally per executed operation,
//! labelled by operation type and root field.
//!
//! # Where the counts are taken
//!
//! `async-graphql` drives the two transports through different hook sets, and neither hook alone
//! covers both:
//!
//! * HTTP (`Schema::execute` / `execute_batch`) calls [`Extension::request`] around the whole
//!   operation — parse, validate and execute.
//! * WebSocket (`Schema::execute_stream_with_session_data`, which the async-graphql WebSocket
//!   transport uses for *every* operation, subscriptions included) never calls `request`; it calls
//!   [`Extension::subscribe`] once, around the response stream.
//!
//! So the HTTP count is taken in `request` and the WebSocket count in `subscribe`, which yields
//! exactly one count per operation on each transport with no double counting. The `execute` hook —
//! the obvious first choice — is deliberately *not* used: it is skipped entirely for subscription
//! operations (so subscriptions would be invisible) and it is also skipped whenever parse or
//! validation fails (so malformed operations, the ones an operator most wants to see, would be
//! invisible too).
//!
//! A subscription is counted **once, at subscription start**, not once per emitted event: the
//! stream wrapper records as soon as the operation shape is known (the first poll, which is where
//! `async-graphql` parses and resolves the subscription streams) and never again. Its duration
//! sample is therefore the subscription's set-up cost, not the lifetime of the connection — a
//! long-lived subscription must not pour idle time into the latency histogram. Queries and
//! mutations sent over the WebSocket are counted when their single response arrives, so their
//! duration is the full operation.
//!
//! # Label cardinality
//!
//! `root_field` is the first top-level field of the executed operation and is checked against the
//! schema registry before it is used as a label: a client can send `{ notAField }`, which parses
//! fine and only fails validation, so an unchecked label would let any client mint unbounded
//! series. Anything the schema does not define collapses to `unknown`, as does an operation whose
//! document could not be parsed or whose operation could not be selected. Operation *names* are
//! client-controlled free text and are never used as labels.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::time::Instant;

use async_graphql::extensions::{
    Extension, ExtensionContext, ExtensionFactory, NextParseQuery, NextPrepareRequest, NextRequest,
    NextSubscribe,
};
use async_graphql::futures_util::stream::{BoxStream, Stream};
use async_graphql::parser::types::{
    DocumentOperations, ExecutableDocument, OperationType, Selection,
};
use async_graphql::registry::Registry;
use async_graphql::{Request, Response, ServerResult, Variables};
use metrics::{Unit, counter, describe_counter, describe_histogram, histogram};

/// Label value used whenever the operation shape could not be established.
const UNKNOWN: &str = "unknown";

/// `operation_type` label values.
const QUERY: &str = "query";
const MUTATION: &str = "mutation";
const SUBSCRIPTION: &str = "subscription";

/// Introspection meta-fields, which are valid root fields but live outside the type registry.
const INTROSPECTION_ROOT_FIELDS: [&str; 3] = ["__schema", "__type", "__typename"];

/// Registers HELP text for every `scryer_graphql_*` family.
///
/// Must be called with the target recorder already installed — `describe_*!` against the no-op
/// recorder silently loses the text.
pub fn describe_graphql_metrics() {
    describe_counter!(
        "scryer_graphql_operations_total",
        "GraphQL operations executed, labelled by operation type, root field and whether the response carried errors. Subscriptions are counted once at subscription start, not per emitted event."
    );
    describe_histogram!(
        "scryer_graphql_operation_duration_seconds",
        Unit::Seconds,
        "Wall-clock duration of one GraphQL operation including parsing and validation. For subscriptions this is the set-up cost, not the lifetime of the subscription."
    );
    describe_counter!(
        "scryer_graphql_errors_total",
        "Errors carried by GraphQL responses, labelled by operation type and root field. Incremented by the number of errors in the response, so it also registers a zero series for successful operations."
    );
}

/// The shape of an operation, reduced to two bounded labels.
#[derive(Clone, Debug, Eq, PartialEq)]
struct OperationShape {
    operation_type: &'static str,
    root_field: String,
}

impl OperationShape {
    /// The shape used when the document could not be parsed or the operation could not be picked.
    fn unknown() -> Self {
        Self {
            operation_type: UNKNOWN,
            root_field: UNKNOWN.to_owned(),
        }
    }

    fn is_subscription(&self) -> bool {
        self.operation_type == SUBSCRIPTION
    }
}

/// Emits the three families for one completed (or started) operation.
fn record_operation(shape: &OperationShape, elapsed_seconds: f64, errors: usize) {
    let status = if errors == 0 { "ok" } else { "error" };
    counter!(
        "scryer_graphql_operations_total",
        "operation_type" => shape.operation_type,
        "root_field" => shape.root_field.clone(),
        "status" => status,
    )
    .increment(1);
    histogram!(
        "scryer_graphql_operation_duration_seconds",
        "operation_type" => shape.operation_type,
        "root_field" => shape.root_field.clone(),
    )
    .record(elapsed_seconds);
    // Incremented unconditionally so a healthy operation registers its zero series rather than
    // leaving a gap that only appears once something fails.
    counter!(
        "scryer_graphql_errors_total",
        "operation_type" => shape.operation_type,
        "root_field" => shape.root_field.clone(),
    )
    .increment(errors as u64);
}

/// Maps an operation type to its bounded label value.
fn operation_type_label(ty: OperationType) -> &'static str {
    match ty {
        OperationType::Query => QUERY,
        OperationType::Mutation => MUTATION,
        OperationType::Subscription => SUBSCRIPTION,
    }
}

/// Returns whether `field` is a root field the schema actually defines for `ty`.
///
/// This is what keeps `root_field` bounded by the schema rather than by client input.
fn is_known_root_field(registry: &Registry, ty: OperationType, field: &str) -> bool {
    if INTROSPECTION_ROOT_FIELDS.contains(&field) {
        return true;
    }
    let root_type = match ty {
        OperationType::Query => Some(registry.query_type.as_str()),
        OperationType::Mutation => registry.mutation_type.as_deref(),
        OperationType::Subscription => registry.subscription_type.as_deref(),
    };
    root_type
        .and_then(|name| registry.types.get(name))
        .and_then(|meta| meta.field_by_name(field))
        .is_some()
}

/// Derives the operation shape from a parsed document.
///
/// `operation_name` is the name the *request* asked for, which is how `async-graphql` itself picks
/// the operation out of a multi-operation document; it is used only for selection and never
/// becomes a label.
fn derive_shape(
    registry: &Registry,
    document: &ExecutableDocument,
    operation_name: Option<&str>,
) -> OperationShape {
    let operation = match (&document.operations, operation_name) {
        (DocumentOperations::Single(operation), _) => Some(operation),
        (DocumentOperations::Multiple(operations), Some(name)) => operations
            .iter()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, operation)| operation),
        (DocumentOperations::Multiple(operations), None) if operations.len() == 1 => {
            operations.values().next()
        }
        // A multi-operation document with no operation name is an error in async-graphql; there is
        // no single shape to attribute the request to.
        (DocumentOperations::Multiple(_), None) => None,
    };

    let Some(operation) = operation else {
        return OperationShape::unknown();
    };

    let ty = operation.node.ty;
    let root_field = operation
        .node
        .selection_set
        .node
        .items
        .iter()
        .find_map(|selection| match &selection.node {
            Selection::Field(field) => Some(field.node.name.node.as_str()),
            // A leading fragment spread or inline fragment has no field name of its own; resolving
            // it would mean walking the fragment table for a label, which is not worth it.
            Selection::FragmentSpread(_) | Selection::InlineFragment(_) => None,
        })
        .filter(|field| is_known_root_field(registry, ty, field))
        .unwrap_or(UNKNOWN);

    OperationShape {
        operation_type: operation_type_label(ty),
        root_field: root_field.to_owned(),
    }
}

/// Installs [`GraphqlMetricsInstance`] on a schema.
pub struct GraphqlMetricsExtension;

impl ExtensionFactory for GraphqlMetricsExtension {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(GraphqlMetricsInstance::default())
    }
}

/// Per-operation extension state.
///
/// `async-graphql` creates one instance per request (and one per subscription stream), so the
/// interior mutability here is scoped to a single operation.
#[derive(Default)]
struct GraphqlMetricsInstance {
    /// The operation name the request asked for, captured before the document is parsed.
    operation_name: Mutex<Option<String>>,
    /// The derived shape, populated once the document has been parsed.
    shape: Arc<Mutex<Option<OperationShape>>>,
}

impl GraphqlMetricsInstance {
    fn operation_name(&self) -> Option<String> {
        self.operation_name
            .lock()
            .expect("graphql metrics operation-name lock must not be poisoned")
            .clone()
    }

    fn set_shape(&self, shape: OperationShape) {
        *self
            .shape
            .lock()
            .expect("graphql metrics shape lock must not be poisoned") = Some(shape);
    }

    fn shape(&self) -> OperationShape {
        self.shape
            .lock()
            .expect("graphql metrics shape lock must not be poisoned")
            .clone()
            .unwrap_or_else(OperationShape::unknown)
    }
}

#[async_graphql::async_trait::async_trait]
impl Extension for GraphqlMetricsInstance {
    /// Captures the requested operation name before parsing, so a multi-operation document can be
    /// narrowed to the operation that actually runs.
    async fn prepare_request(
        &self,
        ctx: &ExtensionContext<'_>,
        request: Request,
        next: NextPrepareRequest<'_>,
    ) -> ServerResult<Request> {
        *self
            .operation_name
            .lock()
            .expect("graphql metrics operation-name lock must not be poisoned") =
            request.operation_name.clone();
        next.run(ctx, request).await
    }

    /// Records the operation shape as soon as the document is available.
    async fn parse_query(
        &self,
        ctx: &ExtensionContext<'_>,
        query: &str,
        variables: &Variables,
        next: NextParseQuery<'_>,
    ) -> ServerResult<ExecutableDocument> {
        let result = next.run(ctx, query, variables).await;
        let shape = match &result {
            Ok(document) => derive_shape(
                &ctx.schema_env.registry,
                document,
                self.operation_name().as_deref(),
            ),
            Err(_) => OperationShape::unknown(),
        };
        self.set_shape(shape);
        result
    }

    /// HTTP transport: one count per operation, covering parse, validation and execution.
    async fn request(&self, ctx: &ExtensionContext<'_>, next: NextRequest<'_>) -> Response {
        let started = Instant::now();
        let response = next.run(ctx).await;
        record_operation(
            &self.shape(),
            started.elapsed().as_secs_f64(),
            response.errors.len(),
        );
        response
    }

    /// WebSocket transport: one count per operation, taken from the response stream.
    fn subscribe<'s>(
        &self,
        ctx: &ExtensionContext<'_>,
        stream: BoxStream<'s, Response>,
        next: NextSubscribe<'_>,
    ) -> BoxStream<'s, Response> {
        Box::pin(MeteredSubscription {
            inner: next.run(ctx, stream),
            shape: self.shape.clone(),
            started: Instant::now(),
            recorded: false,
        })
    }
}

/// Response stream that records its operation exactly once.
struct MeteredSubscription<'s> {
    inner: BoxStream<'s, Response>,
    shape: Arc<Mutex<Option<OperationShape>>>,
    started: Instant,
    recorded: bool,
}

impl MeteredSubscription<'_> {
    /// The shape parsed inside the stream body, or `None` if parsing has not happened yet.
    fn shape(&self) -> Option<OperationShape> {
        self.shape
            .lock()
            .expect("graphql metrics shape lock must not be poisoned")
            .clone()
    }

    fn record(&mut self, response: Option<&Response>) {
        self.recorded = true;
        let shape = self.shape().unwrap_or_else(OperationShape::unknown);
        let errors = response.map_or(0, |response| response.errors.len());
        record_operation(&shape, self.started.elapsed().as_secs_f64(), errors);
    }
}

impl Stream for MeteredSubscription<'_> {
    type Item = Response;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        // Every field is `Unpin` (a `Pin<Box<..>>` stream, an `Arc`, an `Instant`, a `bool`), so
        // the wrapper can be moved out of its pin to poll the inner stream.
        let this = self.get_mut();
        let polled = this.inner.as_mut().poll_next(cx);
        if !this.recorded {
            match &polled {
                // A query or mutation over the WebSocket: its single response is the whole
                // operation, so this is also its completion.
                Poll::Ready(Some(response)) => this.record(Some(response)),
                Poll::Ready(None) => this.record(None),
                Poll::Pending => {
                    // A subscription that is now waiting for events: parsing and stream
                    // resolution are done, so the shape is known and the subscription has
                    // started. Count it here rather than on its first event, which may never
                    // arrive.
                    if this.shape().is_some_and(|shape| shape.is_subscription()) {
                        this.record(None);
                    }
                }
            }
        }
        polled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_graphql::futures_util::StreamExt;
    use async_graphql::{EmptyMutation, Object, Schema, Subscription};
    use metrics::{Key, with_local_recorder};
    use metrics_util::MetricKind;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    struct Query;

    #[Object]
    impl Query {
        async fn ok(&self) -> &'static str {
            "ok"
        }

        async fn boom(&self) -> async_graphql::Result<&'static str> {
            Err(async_graphql::Error::new("boom"))
        }
    }

    struct SubscriptionRoot;

    #[Subscription]
    impl SubscriptionRoot {
        async fn ticks(&self) -> impl Stream<Item = i32> {
            async_graphql::futures_util::stream::iter(vec![1, 2, 3])
        }

        /// A subscription that never emits, standing in for the common real case: a client
        /// subscribes and then waits.
        async fn never(&self) -> impl Stream<Item = i32> {
            async_graphql::futures_util::stream::pending()
        }
    }

    type TestSchema = Schema<Query, EmptyMutation, SubscriptionRoot>;

    fn test_schema() -> TestSchema {
        Schema::build(Query, EmptyMutation, SubscriptionRoot)
            .extension(GraphqlMetricsExtension)
            .finish()
    }

    type SnapshotEntries = Vec<(
        metrics_util::CompositeKey,
        Option<Unit>,
        Option<metrics::SharedString>,
        DebugValue,
    )>;

    fn find(
        snapshot: &SnapshotEntries,
        kind: MetricKind,
        name: &str,
        labels: &[(&str, &str)],
    ) -> Option<DebugValue> {
        snapshot.iter().find_map(|(composite, _, _, value)| {
            if composite.kind() != kind {
                return None;
            }
            let key: &Key = composite.key();
            if key.name() != name {
                return None;
            }
            let actual: Vec<(String, String)> = key
                .labels()
                .map(|label| (label.key().to_owned(), label.value().to_owned()))
                .collect();
            let expected: Vec<(String, String)> = labels
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect();
            if actual != expected {
                return None;
            }
            Some(match value {
                DebugValue::Counter(count) => DebugValue::Counter(*count),
                DebugValue::Gauge(gauge) => DebugValue::Gauge(*gauge),
                DebugValue::Histogram(samples) => DebugValue::Histogram(samples.clone()),
            })
        })
    }

    /// Runs `body` on a current-thread runtime with a local recorder installed, so nothing
    /// escapes to another thread where the recorder would not be visible.
    fn snapshot_of<F>(body: F) -> SnapshotEntries
    where
        F: FnOnce(&tokio::runtime::Runtime),
    {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime builds");

        with_local_recorder(&recorder, || body(&runtime));

        snapshotter.snapshot().into_vec()
    }

    #[test]
    fn successful_query_is_counted_ok() {
        let snapshot = snapshot_of(|runtime| {
            let schema = test_schema();
            let response = runtime.block_on(schema.execute("{ ok }"));
            assert!(response.errors.is_empty(), "{:?}", response.errors);
        });

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_operations_total",
                &[
                    ("operation_type", "query"),
                    ("root_field", "ok"),
                    ("status", "ok"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "expected one ok query, got:\n{snapshot:?}"
        );
        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_errors_total",
                &[("operation_type", "query"), ("root_field", "ok")],
            ),
            Some(DebugValue::Counter(0)),
            "expected a zero error series, got:\n{snapshot:?}"
        );
        match find(
            &snapshot,
            MetricKind::Histogram,
            "scryer_graphql_operation_duration_seconds",
            &[("operation_type", "query"), ("root_field", "ok")],
        ) {
            Some(DebugValue::Histogram(samples)) => assert_eq!(samples.len(), 1),
            other => panic!("expected one duration sample, got {other:?}"),
        }
    }

    #[test]
    fn failing_query_is_counted_error_with_error_tally() {
        let snapshot = snapshot_of(|runtime| {
            let schema = test_schema();
            let response = runtime.block_on(schema.execute("{ boom }"));
            assert_eq!(response.errors.len(), 1);
        });

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_operations_total",
                &[
                    ("operation_type", "query"),
                    ("root_field", "boom"),
                    ("status", "error"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "expected one errored query, got:\n{snapshot:?}"
        );
        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_errors_total",
                &[("operation_type", "query"), ("root_field", "boom")],
            ),
            Some(DebugValue::Counter(1)),
            "expected one counted error, got:\n{snapshot:?}"
        );
    }

    #[test]
    fn unknown_root_fields_do_not_become_labels() {
        let snapshot = snapshot_of(|runtime| {
            let schema = test_schema();
            let response = runtime.block_on(schema.execute("{ definitelyNotAField }"));
            assert!(!response.errors.is_empty());
        });

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_operations_total",
                &[
                    ("operation_type", "query"),
                    ("root_field", "unknown"),
                    ("status", "error"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "a field the schema does not define must collapse to unknown, got:\n{snapshot:?}"
        );
    }

    #[test]
    fn unparseable_documents_are_counted_as_unknown() {
        let snapshot = snapshot_of(|runtime| {
            let schema = test_schema();
            let response = runtime.block_on(schema.execute("{ ok"));
            assert!(!response.errors.is_empty());
        });

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_operations_total",
                &[
                    ("operation_type", "unknown"),
                    ("root_field", "unknown"),
                    ("status", "error"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "expected an unknown/unknown count, got:\n{snapshot:?}"
        );
    }

    #[test]
    fn named_operation_is_selected_from_a_multi_operation_document() {
        let snapshot = snapshot_of(|runtime| {
            let schema = test_schema();
            let request =
                async_graphql::Request::new("query A { ok } query B { boom }").operation_name("B");
            let response = runtime.block_on(schema.execute(request));
            assert_eq!(response.errors.len(), 1);
        });

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_operations_total",
                &[
                    ("operation_type", "query"),
                    ("root_field", "boom"),
                    ("status", "error"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "expected the named operation's shape, got:\n{snapshot:?}"
        );
    }

    #[test]
    fn subscription_is_counted_once_not_per_event() {
        let snapshot = snapshot_of(|runtime| {
            let schema = test_schema();
            runtime.block_on(async {
                let mut stream = schema.execute_stream("subscription { ticks }");
                let mut events = 0;
                while let Some(response) = stream.next().await {
                    assert!(response.errors.is_empty(), "{:?}", response.errors);
                    events += 1;
                }
                assert_eq!(events, 3, "expected three emitted events");
            });
        });

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_operations_total",
                &[
                    ("operation_type", "subscription"),
                    ("root_field", "ticks"),
                    ("status", "ok"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "a subscription must be counted once, not per event, got:\n{snapshot:?}"
        );
    }

    #[test]
    fn subscription_is_counted_before_its_first_event() {
        let snapshot = snapshot_of(|runtime| {
            let schema = test_schema();
            runtime.block_on(async {
                let mut stream = schema.execute_stream("subscription { never }");
                let outcome =
                    tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await;
                assert!(outcome.is_err(), "the subscription must not emit anything");
            });
        });

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_operations_total",
                &[
                    ("operation_type", "subscription"),
                    ("root_field", "never"),
                    ("status", "ok"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "a subscription must be counted at start, not on its first event, got:\n{snapshot:?}"
        );
    }

    #[test]
    fn websocket_query_is_counted_once() {
        let snapshot = snapshot_of(|runtime| {
            let schema = test_schema();
            runtime.block_on(async {
                let mut stream = schema.execute_stream("{ ok }");
                let mut responses = 0;
                while let Some(response) = stream.next().await {
                    assert!(response.errors.is_empty(), "{:?}", response.errors);
                    responses += 1;
                }
                assert_eq!(responses, 1);
            });
        });

        assert_eq!(
            find(
                &snapshot,
                MetricKind::Counter,
                "scryer_graphql_operations_total",
                &[
                    ("operation_type", "query"),
                    ("root_field", "ok"),
                    ("status", "ok"),
                ],
            ),
            Some(DebugValue::Counter(1)),
            "a query over the stream transport must be counted exactly once, got:\n{snapshot:?}"
        );
    }

    #[test]
    fn describe_graphql_metrics_registers_help_text() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        let snapshot = with_local_recorder(&recorder, || {
            describe_graphql_metrics();
            counter!(
                "scryer_graphql_operations_total",
                "operation_type" => "query",
                "root_field" => "ok",
                "status" => "ok",
            )
            .increment(1);
            snapshotter.snapshot().into_vec()
        });

        let description = snapshot
            .iter()
            .find(|(composite, _, _, _)| {
                composite.key().name() == "scryer_graphql_operations_total"
            })
            .and_then(|(_, _, description, _)| description.clone())
            .unwrap_or_else(|| panic!("expected a description, got:\n{snapshot:?}"));
        assert!(!description.is_empty());
    }
}
