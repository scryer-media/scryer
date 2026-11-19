//! Every operator-skill operation file must parse and validate against the
//! schema this binary serves. The files are run by agents against live
//! instances, and the MCP catalog embeds them, so a schema change that breaks
//! one fails here instead of in the field.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_graphql::extensions::{Extension, ExtensionContext, ExtensionFactory, NextExecute};
use async_graphql::{Request, Response, Schema};
use scryer_interface::mutation::MutationRoot;
use scryer_interface::query::QueryRoot;
use scryer_interface::subscription::SubscriptionRoot;

/// Stops every request after validation, so no resolver runs and only parse
/// and validation errors can come back.
struct ValidateOnly;

impl ExtensionFactory for ValidateOnly {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(ValidateOnlyExtension)
    }
}

struct ValidateOnlyExtension;

#[async_trait::async_trait]
impl Extension for ValidateOnlyExtension {
    async fn execute(
        &self,
        _ctx: &ExtensionContext<'_>,
        _operation_name: Option<&str>,
        _next: NextExecute<'_>,
    ) -> Response {
        Response::default()
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn operation_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for skill in std::fs::read_dir(root.join("skills")).expect("skills directory") {
        let operations = skill.expect("skill entry").path().join("operations");
        let Ok(entries) = std::fs::read_dir(&operations) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("operation entry").path();
            if path
                .extension()
                .is_some_and(|extension| extension == "graphql")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[tokio::test]
async fn every_skill_operation_file_validates_against_the_served_schema() {
    let schema = Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        SubscriptionRoot,
    )
    .limit_recursive_depth(scryer_interface::GRAPHQL_RECURSIVE_DEPTH_LIMIT)
    .extension(ValidateOnly)
    .finish();

    let root = repository_root();
    let files = operation_files(&root);
    assert!(
        files
            .iter()
            .any(|path| path.ends_with("scryer-connect/operations/Me.graphql")),
        "the operation files the MCP catalog embeds must be found"
    );

    let mut failures = Vec::new();
    for path in &files {
        let document = std::fs::read_to_string(path).expect("read operation file");
        let response = schema.execute(Request::new(document)).await;
        if !response.errors.is_empty() {
            let relative = path.strip_prefix(&root).unwrap_or(path);
            let messages = response
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            failures.push(format!("{}: {messages}", relative.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} operation files do not validate:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

fn validating_schema() -> Schema<QueryRoot, MutationRoot, SubscriptionRoot> {
    Schema::build(
        QueryRoot::default(),
        MutationRoot::default(),
        SubscriptionRoot,
    )
    .limit_recursive_depth(scryer_interface::GRAPHQL_RECURSIVE_DEPTH_LIMIT)
    .extension(ValidateOnly)
    .finish()
}

/// Every catalog document, each variant of a name with differing texts
/// included.
fn catalog_documents() -> Vec<(&'static str, &'static str)> {
    super::catalog::CATALOG
        .operations()
        .iter()
        .map(|operation| (operation.name.as_str(), operation.text))
        .collect()
}

#[tokio::test]
async fn every_catalog_operation_validates_against_the_served_schema() {
    let schema = validating_schema();
    let documents = catalog_documents();
    assert!(!documents.is_empty());
    for name in &super::catalog::CATALOG.varied {
        let variants = documents
            .iter()
            .filter(|(document_name, _)| document_name == name)
            .count();
        assert!(variants > 1, "every variant of {name} is checked");
    }
    let mut failures = Vec::new();
    for (name, document) in &documents {
        let response = schema
            .execute(Request::new(*document).operation_name(*name))
            .await;
        if !response.errors.is_empty() {
            let messages = response
                .errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            failures.push(format!("{name}: {messages}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} catalog operations do not validate:\n{}",
        failures.len(),
        documents.len(),
        failures.join("\n")
    );
}

#[test]
fn no_catalog_operation_is_an_authentication_operation() {
    // Tool calls are charged once, before the tool runs, and their operations
    // run without the authentication limiter; that holds only while no
    // catalog operation authenticates.
    let analyze =
        |document: &str| crate::rate_limit::analyze_authentication_request(&Request::new(document));
    let control = analyze("mutation Fixture { login(input: {}) { token } }");
    assert!(control.class.is_some(), "the check must see a login");
    for (name, document) in catalog_documents() {
        let analysis = analyze(document);
        assert!(
            analysis.class.is_none() && !analysis.rejected,
            "{name} is an authentication operation or could not be read"
        );
    }
}

#[tokio::test]
async fn a_document_naming_an_unknown_field_fails_validation() {
    // Guards the check itself: a document the schema does not serve must not
    // pass because execution was skipped.
    let schema = validating_schema();
    let response = schema
        .execute(Request::new(
            "query Fixture { me { notAFieldScryerServes } }",
        ))
        .await;
    assert!(!response.errors.is_empty());
}
