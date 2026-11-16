use crate::{lifecycle, request, rewrite_package_declaration, runtime, validation};
use serde_json::Value;

fn catalog() -> Value {
    serde_json::from_str(include_str!(
        "../../../apps/scryer-web/lib/contracts/rego-assistance.json"
    ))
    .expect("editor catalog is JSON")
}

fn snippet_defaults(source: &str) -> String {
    let mut result = String::new();
    let mut rest = source;
    while let Some((before, after)) = rest.split_once("${") {
        result.push_str(before);
        let (value, remaining) = after.split_once('}').expect("closed snippet placeholder");
        let (_, default) = value
            .split_once(':')
            .expect("numbered placeholder with default");
        result.push_str(default);
        rest = remaining;
    }
    result.push_str(rest);
    result
}

#[test]
fn rego_editor_snippets_pass_their_family_validator() {
    for snippet in catalog()["snippets"].as_array().unwrap() {
        let source = snippet_defaults(snippet["source"].as_str().unwrap());
        let id = "editor_snippet";
        let result = match snippet["family"].as_str().unwrap() {
            "release" => {
                validation::validate_user_rule(&rewrite_package_declaration(&source, id), id)
            }
            "request" => validation::validate_request_rule(
                &request::rewrite_package_declaration(&source, id),
                id,
            ),
            "lifecycle" => validation::validate_lifecycle_rule(
                &lifecycle::rewrite_package_declaration(&source, id),
                id,
            ),
            family => panic!("unknown editor family {family}"),
        }
        .expect("validator completes");
        assert!(result.valid, "{}: {:?}", snippet["label"], result.errors);
    }
}

#[test]
fn rego_editor_helper_signatures_match_the_registered_runtime() {
    let mut engine = runtime::configured_engine(&runtime::RuntimeLimits::release_defaults());
    for helper in catalog()["helpers"].as_array().unwrap() {
        let name = helper["name"].as_str().unwrap();
        let arguments: Vec<&str> = helper["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| match argument.as_str().unwrap() {
                "bytes" => "1073741824",
                "object" => "{}",
                "default" => "null",
                _ => "\"eng\"",
            })
            .collect();
        let expression = format!("{name}({})", arguments.join(", "));
        let result = engine
            .eval_query(expression.clone(), false)
            .expect(&expression);
        assert!(!result.result.is_empty(), "undefined helper: {expression}");
    }
}
