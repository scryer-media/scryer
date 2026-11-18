//! Every operator-skill operation file must parse and validate against the
//! schema this binary serves. The files are run by agents against live
//! instances, and MCP tools embed them, so a schema change that breaks one
//! fails here instead of in the field.

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
        "the operation files the MCP tools embed must be found"
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
