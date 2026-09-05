use crate::maintenance::{self, USER_PACKAGE_PREFIX as MAINTENANCE_USER_PACKAGE_PREFIX};
use crate::runtime::{self, RuntimeLimits};
use crate::{
    AudioStreamDoc, BuiltinScoreDoc, ContextDoc, FileDoc, ProfileDoc, ReleaseDoc, RulesError,
    SubtitleStreamDoc, UserRuleInput, score_entry_wrapper_policy_path,
    score_entry_wrapper_rule_path, score_entry_wrapper_source,
};
use regorus::{
    Value,
    unstable::{Expr, Literal, Module, Parser, Query, Rule, RuleBody, RuleHead, Source},
};
use std::{
    collections::{BTreeSet, HashSet},
    sync::OnceLock,
};

#[derive(Debug, serde::Deserialize)]
struct RuleInputContract {
    sections: Vec<RuleInputContractSection>,
}

#[derive(Debug, serde::Deserialize)]
struct RuleInputContractSection {
    path: String,
    fields: Vec<RuleInputContractField>,
}

#[derive(Debug, serde::Deserialize)]
struct RuleInputContractField {
    field: String,
    #[serde(rename = "type")]
    field_type: String,
}

#[derive(Debug)]
struct RuleInputCatalog {
    known_paths: HashSet<String>,
    array_container_paths: HashSet<String>,
}

/// Everything the input-path walker needs to judge one policy family's paths.
///
/// The release family alone allows `input.release.extra.<key>`, whose keys come
/// from indexer-supplied attributes and so cannot be catalogued.
#[derive(Debug, Clone, Copy)]
struct InputPathContext {
    catalog: &'static RuleInputCatalog,
    allow_release_extra: bool,
    /// Maintenance only: every `input.facts.<name>` must name its fact with a
    /// literal, because the engine reads that set to decide whether the subject
    /// is even knowable enough to consult the rule.
    static_facts_only: bool,
}

impl InputPathContext {
    fn release() -> Self {
        Self {
            catalog: release_input_catalog(),
            allow_release_extra: true,
            static_facts_only: false,
        }
    }

    fn maintenance() -> Self {
        Self {
            catalog: maintenance_input_catalog(),
            allow_release_extra: false,
            static_facts_only: true,
        }
    }
}

/// Result of validating a user-authored Rego rule.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

impl ValidationResult {
    pub fn valid() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            valid: false,
            errors: vec![message.into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputPathComponent {
    Field(String),
    ArrayItem,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputReferencePath {
    components: Vec<InputPathComponent>,
    display: String,
}

impl InputReferencePath {
    fn normalized(&self) -> String {
        let mut out = String::new();
        for component in &self.components {
            match component {
                InputPathComponent::Field(field) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(field);
                }
                InputPathComponent::ArrayItem => out.push_str("[]"),
                InputPathComponent::Dynamic => out.push_str("[*]"),
            }
        }
        out
    }

    fn is_dynamic_extra_path(&self) -> bool {
        matches!(
            self.components.as_slice(),
            [
                InputPathComponent::Field(input),
                InputPathComponent::Field(release),
                InputPathComponent::Field(extra),
                ..,
            ] if input == "input" && release == "release" && extra == "extra"
        )
    }

    fn has_dynamic_component(&self) -> bool {
        self.components
            .iter()
            .any(|component| matches!(component, InputPathComponent::Dynamic))
    }

    /// The fact this path reads, when it reads one by name.
    ///
    /// `input.facts.tags[0]` and `input.facts.files[_].quality` both name
    /// `tags` / `files`; `input.facts` on its own names nothing.
    fn maintenance_fact_name(&self) -> Option<&str> {
        match self.components.as_slice() {
            [
                InputPathComponent::Field(input),
                InputPathComponent::Field(facts),
                InputPathComponent::Field(name),
                ..,
            ] if input == "input" && facts == "facts" => Some(name.as_str()),
            _ => None,
        }
    }

    /// True when the path selects a fact with something other than a literal
    /// name — `input.facts[key]`, `input.facts[_]`, `input.facts[i]`.
    fn selects_fact_dynamically(&self) -> bool {
        matches!(
            self.components.as_slice(),
            [
                InputPathComponent::Field(input),
                InputPathComponent::Field(facts),
                selector,
                ..,
            ] if input == "input"
                && facts == "facts"
                && !matches!(selector, InputPathComponent::Field(_))
        )
    }

    /// True when the path reads the whole `input.facts` object without naming
    /// a fact — e.g. `object.get(input.facts, "x", false)` or
    /// `count(input.facts)`. Such a read reaches facts the referenced-fact set
    /// never sees, so an unknown fact would be read as absent instead of
    /// holding the rule.
    fn reads_facts_object_wholesale(&self) -> bool {
        matches!(
            self.components.as_slice(),
            [
                InputPathComponent::Field(input),
                InputPathComponent::Field(facts),
            ] if input == "input" && facts == "facts"
        )
    }

    /// True when the path is a bare `input` — the whole document handed around
    /// as one value, e.g. `f := input`, `object.get(input, ["facts", "x"],
    /// false)`, `walk(input, [p, v])` or `with input as {...}`. Because the
    /// walker emits maximal paths, `input.facts.monitored` never produces this;
    /// only a genuinely standalone `input` does.
    fn references_input_wholesale(&self) -> bool {
        matches!(
            self.components.as_slice(),
            [InputPathComponent::Field(input)] if input == "input"
        )
    }
}

fn build_input_catalog(contract_json: &str, contract_name: &str) -> RuleInputCatalog {
    let contract: RuleInputContract = serde_json::from_str(contract_json)
        .unwrap_or_else(|e| panic!("{contract_name} should be valid: {e}"));

    let mut known_paths = HashSet::new();
    let mut array_container_paths = HashSet::new();
    known_paths.insert("input".to_string());

    for section in contract.sections {
        known_paths.insert(section.path.clone());
        if let Some(base_path) = section.path.strip_suffix("[]") {
            array_container_paths.insert(base_path.to_string());
        }

        for field in section.fields {
            let field_path = format!("{}.{}", section.path, field.field);
            known_paths.insert(field_path.clone());
            if field.field_type.ends_with("[]") {
                array_container_paths.insert(field_path.clone());
                known_paths.insert(format!("{field_path}[]"));
            }
        }
    }

    RuleInputCatalog {
        known_paths,
        array_container_paths,
    }
}

fn release_input_catalog() -> &'static RuleInputCatalog {
    static CATALOG: OnceLock<RuleInputCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        build_input_catalog(
            include_str!("../rule-input-contract.json"),
            "rule-input-contract.json",
        )
    })
}

fn maintenance_input_catalog() -> &'static RuleInputCatalog {
    static CATALOG: OnceLock<RuleInputCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        build_input_catalog(
            include_str!("../maintenance-input-contract.json"),
            "maintenance-input-contract.json",
        )
    })
}

fn unknown_rule_input_path_message(path: &str) -> String {
    match path {
        "input.release.password_protected" => format!(
            "Unknown rule input path '{path}'. Use a documented field from the Rules Context Reference. For password-protected releases, use 'input.release.is_password_protected'."
        ),
        _ => format!(
            "Unknown rule input path '{path}'. Use one of the documented input fields from the Rules Context Reference."
        ),
    }
}

fn dynamic_fact_access_message(path: &str) -> String {
    format!(
        "Unsupported dynamic fact access '{path}'. Name the fact directly (for example \
         input.facts.monitored) — Scryer holds a rule whose facts it could not observe, and it can \
         only do that when it can tell which facts the rule reads."
    )
}

fn whole_facts_object_message(path: &str) -> String {
    format!(
        "Unsupported reference to the whole fact object '{path}'. Name each fact directly (for \
         example input.facts.monitored) — reading the object wholesale would let a rule see an \
         unknown fact as missing instead of being held, and use input.observations for \
         envelope-level access."
    )
}

fn whole_input_document_message(path: &str) -> String {
    format!(
        "Unsupported reference to the whole input document '{path}'. Reference what the rule reads \
         directly (for example input.facts.monitored or input.subject.title_id) — handing the \
         input document around as one value lets a rule reach facts Scryer cannot see it reading, \
         so a fact Scryer could not observe would look absent instead of holding the rule."
    )
}

fn unsupported_dynamic_input_path_message(path: &str) -> String {
    format!(
        "Unsupported dynamic rule input path '{path}'. Use documented field access, documented array indexing, or input.release.extra.<key>."
    )
}

fn parse_module(rego_source: &str, policy_path: &str) -> Result<Module, String> {
    let source = Source::from_contents(policy_path.to_string(), rego_source.to_string())
        .map_err(|e| e.to_string())?;
    let mut parser = Parser::new(&source).map_err(|e| e.to_string())?;
    parser.enable_rego_v1().map_err(|e| e.to_string())?;
    parser.parse().map_err(|e| e.to_string())
}

fn collect_unknown_input_path_errors(
    rego_source: &str,
    policy_path: &str,
    ctx: InputPathContext,
) -> Result<Vec<String>, String> {
    let module = parse_module(rego_source, policy_path)?;
    Ok(module_input_path_errors(&module, ctx))
}

fn module_input_path_errors(module: &Module, ctx: InputPathContext) -> Vec<String> {
    let mut errors = BTreeSet::new();
    walk_module_input_paths(module, ctx, |path| {
        if let Some(error) = validate_input_reference_path(path, ctx) {
            errors.insert(error);
        }
    });
    errors.into_iter().collect()
}

/// Names of the `input.facts.<name>` facts a maintenance policy references.
///
/// This is the compile-time half of host-derived unknownness: the engine holds
/// this set per rule and, at evaluation time, refuses to consult a rule whose
/// referenced facts are not all resolvable for that subject. It is deliberately
/// *static* — [`validate_maintenance_rule`] rejects dynamic fact access,
/// `input` imports and bare `input` references precisely so this set can never
/// be an underestimate of what a rule actually reads.
///
/// References under `input.observations.*` are not collected: that namespace is
/// the opted-out surface where the author takes responsibility for the
/// three-valued envelope themselves.
fn module_referenced_facts(module: &Module) -> Result<BTreeSet<String>, String> {
    let ctx = InputPathContext::maintenance();
    let mut facts = BTreeSet::new();
    let mut error: Option<String> = None;
    walk_module_input_paths(module, ctx, |path| {
        if error.is_some() {
            return;
        }
        // Mirrored here, not only in validation: a stored revision that
        // predates (or somehow bypassed) validation must still never load with
        // an underestimated fact set.
        if path.selects_fact_dynamically() {
            error = Some(dynamic_fact_access_message(&path.display));
        } else if path.reads_facts_object_wholesale() {
            error = Some(whole_facts_object_message(&path.display));
        } else if path.references_input_wholesale() {
            error = Some(whole_input_document_message(&path.display));
        } else if let Some(name) = path.maintenance_fact_name() {
            facts.insert(name.to_string());
        }
    });
    match error {
        Some(error) => Err(error),
        None => Ok(facts),
    }
}

fn walk_module_input_paths(
    module: &Module,
    ctx: InputPathContext,
    mut sink: impl FnMut(&InputReferencePath),
) {
    for rule in &module.policy {
        visit_rule(rule, &mut sink, ctx);
    }
}

/// Facts a maintenance policy reads, plus the parse work needed to find them.
///
/// Fails rather than guessing: a source that will not parse, or that pulls part
/// of `input` in through an import, has no statically resolvable fact set, and
/// a rule whose fact set is unknown must never load.
pub(crate) fn maintenance_fact_references(
    rego_source: &str,
    policy_path: &str,
) -> Result<BTreeSet<String>, String> {
    let module = parse_module(rego_source, policy_path)?;
    if let Some(error) = input_import_error(&module) {
        return Err(error);
    }
    module_referenced_facts(&module)
}

/// The `input.facts.<name>` facts a maintenance matcher reads, for callers
/// outside this crate that must decide something about a rule *before* it runs.
///
/// This is the same static set the engine holds rules on, exposed once so an
/// authorization check and the evaluator can never disagree about what a rule
/// reads. It fails for exactly the sources the engine would refuse to load, so
/// a caller that cannot get an answer here must reject the rule rather than
/// assume it reads nothing.
pub fn maintenance_referenced_facts(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<BTreeSet<String>, RulesError> {
    maintenance_fact_references(rego_source, &maintenance::user_policy_path(rule_set_id))
        .map_err(RulesError::Compilation)
}

/// Reject any import that pulls `input` (or part of it) into scope.
///
/// `import input.facts` would let a rule write `facts.monitored`, which the
/// path walker sees as a plain variable, not a fact reference — the rule would
/// then read facts the host never knew to check for unknownness, silently
/// losing the fail-closed guarantee. Resolving imports back to full paths is
/// tractable, but it means teaching the walker scoping rules (aliases, local
/// bindings that shadow the import) to buy an abbreviation worth one word. The
/// import is refused instead, and the author writes the path out.
fn input_import_error(module: &Module) -> Option<String> {
    module.imports.iter().find_map(|import| {
        (rule_head_name(&import.refr) == Some("input")).then(|| {
            format!(
                "Unsupported import '{}' in a maintenance rule. Reference facts by their full path \
                 (for example input.facts.monitored) so Scryer can tell which facts the rule \
                 depends on.",
                import.refr.span().text().trim()
            )
        })
    })
}

/// Leading name of a rule head reference, e.g. `match` for `match["x"]`.
fn rule_head_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Var { span, .. } => Some(span.text()),
        Expr::RefDot { refr, .. } | Expr::RefBrack { refr, .. } => rule_head_name(refr),
        _ => None,
    }
}

fn module_defines_rule(module: &Module, name: &str) -> bool {
    module.policy.iter().any(|rule| {
        let refr = match rule.as_ref() {
            Rule::Spec { head, .. } => match head {
                RuleHead::Compr { refr, .. }
                | RuleHead::Set { refr, .. }
                | RuleHead::Func { refr, .. } => refr,
            },
            Rule::Default { refr, .. } => refr,
        };
        rule_head_name(refr) == Some(name)
    })
}

fn visit_rule(rule: &Rule, sink: &mut dyn FnMut(&InputReferencePath), ctx: InputPathContext) {
    match rule {
        Rule::Spec { head, bodies, .. } => {
            visit_rule_head(head, sink, ctx);
            for body in bodies {
                visit_rule_body(body, sink, ctx);
            }
        }
        Rule::Default {
            refr, args, value, ..
        } => {
            visit_expr(refr, sink, ctx);
            for arg in args {
                visit_expr(arg, sink, ctx);
            }
            visit_expr(value, sink, ctx);
        }
    }
}

fn visit_rule_head(
    head: &RuleHead,
    sink: &mut dyn FnMut(&InputReferencePath),
    ctx: InputPathContext,
) {
    match head {
        RuleHead::Compr { refr, assign, .. } => {
            visit_expr(refr, sink, ctx);
            if let Some(assign) = assign {
                visit_expr(&assign.value, sink, ctx);
            }
        }
        RuleHead::Set { refr, key, .. } => {
            visit_expr(refr, sink, ctx);
            if let Some(key) = key {
                visit_expr(key, sink, ctx);
            }
        }
        RuleHead::Func {
            refr, args, assign, ..
        } => {
            visit_expr(refr, sink, ctx);
            for arg in args {
                visit_expr(arg, sink, ctx);
            }
            if let Some(assign) = assign {
                visit_expr(&assign.value, sink, ctx);
            }
        }
    }
}

fn visit_rule_body(
    body: &RuleBody,
    sink: &mut dyn FnMut(&InputReferencePath),
    ctx: InputPathContext,
) {
    if let Some(assign) = &body.assign {
        visit_expr(&assign.value, sink, ctx);
    }
    visit_query(&body.query, sink, ctx);
}

fn visit_query(query: &Query, sink: &mut dyn FnMut(&InputReferencePath), ctx: InputPathContext) {
    for stmt in &query.stmts {
        visit_literal(&stmt.literal, sink, ctx);
        for with_mod in &stmt.with_mods {
            visit_expr(&with_mod.refr, sink, ctx);
            visit_expr(&with_mod.r#as, sink, ctx);
        }
    }
}

fn visit_literal(
    literal: &Literal,
    sink: &mut dyn FnMut(&InputReferencePath),
    ctx: InputPathContext,
) {
    match literal {
        Literal::SomeVars { .. } => {}
        Literal::SomeIn {
            key,
            value,
            collection,
            ..
        } => {
            if let Some(key) = key {
                visit_expr(key, sink, ctx);
            }
            visit_expr(value, sink, ctx);
            visit_expr(collection, sink, ctx);
        }
        Literal::Expr { expr, .. } | Literal::NotExpr { expr, .. } => visit_expr(expr, sink, ctx),
        Literal::Every { domain, query, .. } => {
            visit_expr(domain, sink, ctx);
            visit_query(query, sink, ctx);
        }
    }
}

fn visit_expr(expr: &Expr, sink: &mut dyn FnMut(&InputReferencePath), ctx: InputPathContext) {
    // Emit maximal paths only: when this node is itself an input reference,
    // its `refr` chain is a prefix of the same reference and is not re-emitted
    // below. That is what lets a *standalone* `input.facts` — one passed to a
    // builtin rather than followed by a fact name — be told apart from the
    // harmless `input.facts` inside `input.facts.monitored`.
    let extracted = extract_input_reference_path(expr, ctx);
    if let Some(path) = &extracted {
        sink(path);
    }

    match expr {
        Expr::Array { items, .. } | Expr::Set { items, .. } => {
            for item in items {
                visit_expr(item, sink, ctx);
            }
        }
        Expr::Object { fields, .. } => {
            for (_, key, value) in fields {
                visit_expr(key, sink, ctx);
                visit_expr(value, sink, ctx);
            }
        }
        Expr::ArrayCompr { term, query, .. } | Expr::SetCompr { term, query, .. } => {
            visit_expr(term, sink, ctx);
            visit_query(query, sink, ctx);
        }
        Expr::ObjectCompr {
            key, value, query, ..
        } => {
            visit_expr(key, sink, ctx);
            visit_expr(value, sink, ctx);
            visit_query(query, sink, ctx);
        }
        Expr::Call { fcn, params, .. } => {
            visit_expr(fcn, sink, ctx);
            for param in params {
                visit_expr(param, sink, ctx);
            }
        }
        Expr::UnaryExpr { expr, .. } => visit_expr(expr, sink, ctx),
        Expr::RefDot { refr, .. } => {
            if extracted.is_none() {
                visit_expr(refr, sink, ctx);
            }
        }
        Expr::RefBrack { refr, index, .. } => {
            if extracted.is_none() {
                visit_expr(refr, sink, ctx);
            }
            visit_expr(index, sink, ctx);
        }
        Expr::BinExpr { lhs, rhs, .. }
        | Expr::BoolExpr { lhs, rhs, .. }
        | Expr::ArithExpr { lhs, rhs, .. }
        | Expr::AssignExpr { lhs, rhs, .. } => {
            visit_expr(lhs, sink, ctx);
            visit_expr(rhs, sink, ctx);
        }
        Expr::Membership {
            key,
            value,
            collection,
            ..
        } => {
            if let Some(key) = key {
                visit_expr(key, sink, ctx);
            }
            visit_expr(value, sink, ctx);
            visit_expr(collection, sink, ctx);
        }
        Expr::String { .. }
        | Expr::RawString { .. }
        | Expr::Number { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::Var { .. } => {}
        Expr::OrExpr { lhs, rhs, .. } => {
            visit_expr(lhs, sink, ctx);
            visit_expr(rhs, sink, ctx);
        }
    }
}

fn extract_input_reference_path(expr: &Expr, ctx: InputPathContext) -> Option<InputReferencePath> {
    match expr {
        Expr::Var { span, .. } if span.text() == "input" => Some(InputReferencePath {
            components: vec![InputPathComponent::Field("input".to_string())],
            display: "input".to_string(),
        }),
        Expr::RefDot { refr, field, .. } => {
            let mut path = extract_input_reference_path(refr, ctx)?;
            let field_name = field.1.as_string().ok()?.to_string();
            path.display.push('.');
            path.display.push_str(&field_name);
            path.components.push(InputPathComponent::Field(field_name));
            Some(path)
        }
        Expr::RefBrack { refr, index, .. } => {
            let mut path = extract_input_reference_path(refr, ctx)?;
            path.display.push('[');
            path.display.push_str(index.span().text());
            path.display.push(']');

            let current_path = path.normalized();
            if ctx
                .catalog
                .array_container_paths
                .contains(current_path.as_str())
            {
                path.components.push(InputPathComponent::ArrayItem);
            } else {
                match index.as_ref() {
                    Expr::String { value, .. } | Expr::RawString { value, .. } => {
                        let field_name = value.as_string().ok()?.to_string();
                        path.components.push(InputPathComponent::Field(field_name));
                    }
                    Expr::Number { .. } => path.components.push(InputPathComponent::ArrayItem),
                    Expr::Var { span, .. } if span.text() == "_" => {
                        path.components.push(InputPathComponent::ArrayItem);
                    }
                    _ => path.components.push(InputPathComponent::Dynamic),
                }
            }
            Some(path)
        }
        _ => None,
    }
}

fn validate_input_reference_path(
    path: &InputReferencePath,
    ctx: InputPathContext,
) -> Option<String> {
    if ctx.allow_release_extra && path.is_dynamic_extra_path() {
        return None;
    }

    // Fact names must be literal: the engine derives the set of facts a rule
    // depends on from these paths, and a computed name would leave that set
    // silently incomplete.
    if ctx.static_facts_only && path.selects_fact_dynamically() {
        return Some(dynamic_fact_access_message(&path.display));
    }

    if ctx.static_facts_only && path.reads_facts_object_wholesale() {
        return Some(whole_facts_object_message(&path.display));
    }

    // Same reasoning one level up: a bare `input` carries every fact with it,
    // so anything the rule reads out of it afterwards is invisible to the
    // referenced-fact set.
    if ctx.static_facts_only && path.references_input_wholesale() {
        return Some(whole_input_document_message(&path.display));
    }

    if path.has_dynamic_component() {
        return Some(unsupported_dynamic_input_path_message(&path.display));
    }

    let normalized = path.normalized();
    if ctx.catalog.known_paths.contains(normalized.as_str()) {
        None
    } else {
        Some(unknown_rule_input_path_message(&normalized))
    }
}

/// Validate a user-authored Rego rule without persisting it.
///
/// The caller is expected to have already called `rewrite_package_declaration`
/// on the source so the package line matches `rule_set_id`.
///
/// Checks:
/// 1. Package declaration matches `scryer.rules.user.<rule_set_id>`.
/// 2. Source compiles without errors.
/// 3. Dry-run against synthetic input succeeds.
/// 4. Output shape is a map of string keys to integer values.
/// 5. The generated runtime `eval_rule` wrapper evaluates successfully.
pub fn validate_user_rule(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<ValidationResult, RulesError> {
    let expected_pkg = format!("package scryer.rules.user.{rule_set_id}");

    // Check package declaration
    let has_pkg = rego_source.lines().any(|line| line.trim() == expected_pkg);
    if !has_pkg {
        return Ok(ValidationResult::invalid(format!(
            "package declaration must be: {expected_pkg}"
        )));
    }

    // Compile in a throwaway engine
    let mut engine = runtime::configured_engine(&RuntimeLimits::release_defaults());

    let policy_path = format!("user/{rule_set_id}.rego");
    if let Err(e) = engine.add_policy(policy_path.clone(), rego_source.to_string()) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }
    if let Err(e) = engine.add_policy(
        score_entry_wrapper_policy_path(rule_set_id),
        score_entry_wrapper_source(rule_set_id),
    ) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    let input_path_errors =
        collect_unknown_input_path_errors(rego_source, &policy_path, InputPathContext::release())
            .map_err(RulesError::Compilation)?;
    if !input_path_errors.is_empty() {
        return Ok(ValidationResult {
            valid: false,
            errors: input_path_errors,
        });
    }

    // Dry-run against synthetic input
    let test_input = synthetic_test_input();
    let input_value = serde_json::to_value(&test_input).map_err(RulesError::Serialization)?;
    engine.set_input(input_value.into());

    let query = format!("data.scryer.rules.user.{rule_set_id}.score_entry");
    match engine.eval_query(query, false) {
        Ok(results) => {
            let value = results
                .result
                .first()
                .and_then(|r| r.expressions.first())
                .map(|e| &e.value);

            if let Some(v) = value
                && let Err(e) = validate_score_entry_shape(v)
            {
                return Ok(ValidationResult::invalid(format!("output error: {e}")));
            }
            match engine.eval_rule(score_entry_wrapper_rule_path(rule_set_id)) {
                Ok(value) => {
                    if let Err(e) = validate_score_entry_shape(&value) {
                        return Ok(ValidationResult::invalid(format!("output error: {e}")));
                    }
                    Ok(ValidationResult::valid())
                }
                Err(e) => Ok(ValidationResult::invalid(format!("runtime error: {e}"))),
            }
        }
        Err(e) => Ok(ValidationResult::invalid(format!("runtime error: {e}"))),
    }
}

/// Validate that a persisted user rule can execute through the generated
/// runtime `eval_rule` wrapper.
///
/// This deliberately checks only the runtime path. New edits continue to use
/// [`validate_user_rule`] so the editor keeps its richer `eval_query`
/// diagnostics; migrations use this narrower check to protect existing rules
/// when the runtime entry point changes.
pub fn validate_runtime_wrapper(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<ValidationResult, RulesError> {
    let expected_pkg = format!("package scryer.rules.user.{rule_set_id}");
    let has_pkg = rego_source.lines().any(|line| line.trim() == expected_pkg);
    if !has_pkg {
        return Ok(ValidationResult::invalid(format!(
            "package declaration must be: {expected_pkg}"
        )));
    }

    let mut engine = runtime::configured_engine(&RuntimeLimits::release_defaults());

    let policy_path = format!("user/{rule_set_id}.rego");
    if let Err(e) = engine.add_policy(policy_path, rego_source.to_string()) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }
    if let Err(e) = engine.add_policy(
        score_entry_wrapper_policy_path(rule_set_id),
        score_entry_wrapper_source(rule_set_id),
    ) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    let input_value =
        serde_json::to_value(synthetic_test_input()).map_err(RulesError::Serialization)?;
    engine.set_input(input_value.into());

    match engine.eval_rule(score_entry_wrapper_rule_path(rule_set_id)) {
        Ok(value) => match validate_score_entry_shape(&value) {
            Ok(()) => Ok(ValidationResult::valid()),
            Err(e) => Ok(ValidationResult::invalid(format!("output error: {e}"))),
        },
        Err(e) => Ok(ValidationResult::invalid(format!("runtime error: {e}"))),
    }
}

/// Validate a system-managed score-only policy.
///
/// Managed packs are opt-in, so the source is no longer inspected
/// for `scryer.block_score()`. That check only restricted the *spelling* of a
/// veto — a pack could emit the sentinel as a literal and block identically —
/// and the property it was reaching for now lives in `validate_managed_entries`
/// at evaluation time, where it cannot be bypassed.
pub fn validate_managed_rule(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<ValidationResult, RulesError> {
    validate_user_rule(rego_source, rule_set_id)
}

/// Validate a user-authored maintenance matcher without persisting it.
///
/// The caller is expected to have already called
/// [`crate::maintenance::rewrite_package_declaration`] on the source.
///
/// Checks:
/// 1. Package declaration matches `scryer.maintenance.user.<rule_set_id>`.
/// 2. Source and the generated decision wrapper both compile.
/// 3. No import pulls part of `input` into scope, `input` is never referenced
///    as a whole value, and every fact is selected by a literal name. All three
///    keep the referenced-fact set statically resolvable, which is what lets the
///    engine hold a rule whose facts it cannot observe.
/// 4. The source defines a rule named `match`. The wrapper defaults an
///    undefined `match` to false, so a matcher that never defines one would
///    quietly evaluate to no-match forever — this is the only place that catches
///    it.
/// 5. Every `input.*` reference resolves in the maintenance catalog.
/// 6. A dry run against synthetic input yields a well-formed decision.
pub fn validate_maintenance_rule(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<ValidationResult, RulesError> {
    let expected_pkg = format!("package {MAINTENANCE_USER_PACKAGE_PREFIX}.{rule_set_id}");
    if !rego_source.lines().any(|line| line.trim() == expected_pkg) {
        return Ok(ValidationResult::invalid(format!(
            "package declaration must be: {expected_pkg}"
        )));
    }

    let mut engine = runtime::configured_engine(&RuntimeLimits::maintenance_defaults());

    let policy_path = maintenance::user_policy_path(rule_set_id);
    if let Err(e) = engine.add_policy(policy_path.clone(), rego_source.to_string()) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }
    if let Err(e) = engine.add_policy(
        maintenance::decision_wrapper_policy_path(rule_set_id),
        maintenance::decision_wrapper_source(rule_set_id),
    ) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    let module = parse_module(rego_source, &policy_path).map_err(RulesError::Compilation)?;

    if let Some(error) = input_import_error(&module) {
        return Ok(ValidationResult::invalid(error));
    }

    let input_path_errors = module_input_path_errors(&module, InputPathContext::maintenance());
    if !input_path_errors.is_empty() {
        return Ok(ValidationResult {
            valid: false,
            errors: input_path_errors,
        });
    }

    if !module_defines_rule(&module, "match") {
        return Ok(ValidationResult::invalid(
            "maintenance rule must define a boolean 'match' rule, for example: match if { ... }",
        ));
    }

    let input_value = serde_json::to_value(maintenance::synthetic_maintenance_input())
        .map_err(RulesError::Serialization)?;
    engine.set_input(input_value.into());

    match engine.eval_rule(maintenance::decision_wrapper_rule_path(rule_set_id)) {
        Ok(value) => match maintenance::decode_decision(&value) {
            Ok(_) => Ok(ValidationResult::valid()),
            Err(e) => Ok(ValidationResult::invalid(format!("output error: {e}"))),
        },
        Err(e) => Ok(ValidationResult::invalid(format!("runtime error: {e}"))),
    }
}

/// Verify that the evaluation result is a map of string → integer.
/// Floats and out-of-range values are rejected.
fn validate_score_entry_shape(value: &Value) -> Result<(), String> {
    // Value::Undefined means the rule conditions weren't met — valid (no entries)
    if matches!(value, Value::Undefined) {
        return Ok(());
    }

    let obj = value.as_object().map_err(|_| {
        "score_entry must produce an object (map), not a scalar or array".to_string()
    })?;

    for (key, val) in obj.iter() {
        if key.as_string().is_err() {
            return Err(format!("score_entry keys must be strings, got: {key:?}"));
        }
        let key_str = key
            .as_string()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "?".to_string());

        if let Ok(n) = val.as_i64() {
            if i32::try_from(n).is_err() {
                return Err(format!(
                    "score_entry value for {key_str:?} is out of i32 range: {n}"
                ));
            }
        } else if val.as_f64().is_ok() {
            return Err(format!(
                "score_entry values must be integers, got float for key {key_str:?}. \
                 Use round() or ceil() to convert."
            ));
        } else {
            return Err(format!(
                "score_entry values must be integers, got: {val:?} for key {key_str:?}"
            ));
        }
    }

    Ok(())
}

/// Build a representative input for validation dry-runs.
fn synthetic_test_input() -> UserRuleInput {
    UserRuleInput {
        release: ReleaseDoc {
            raw_title: "Test.Movie.2024.2160p.WEB-DL.H.265.DDP.5.1".to_string(),
            quality: Some("2160P".to_string()),
            source: Some("WEB-DL".to_string()),
            video_codec: Some("H.265".to_string()),
            audio: Some("DDP".to_string()),
            audio_codecs: vec!["DDP".to_string()],
            audio_channels: Some("5.1".to_string()),
            languages_audio: vec!["eng".to_string()],
            languages_subtitles: vec!["eng".to_string()],
            is_dual_audio: false,
            is_atmos: false,
            is_dolby_vision: false,
            detected_hdr: false,
            is_remux: false,
            is_bd_disk: false,
            is_proper_upload: false,
            is_repack: false,
            is_ai_enhanced: false,
            is_hardcoded_subs: false,
            is_password_protected: Some(false),
            is_hdr10plus: false,
            is_hlg: false,
            is_10bit: false,
            is_uncensored: false,
            is_dubs_only: false,
            has_release_group: true,
            is_obfuscated: false,
            is_retagged: false,
            streaming_service: None,
            edition: None,
            anime_version: None,
            episode_release_type: Some("single_episode".to_string()),
            is_season_pack: false,
            is_multi_episode: false,
            release_group: Some("TestGroup".to_string()),
            year: Some(2024),
            parse_confidence: 0.9,
            size_bytes: Some(8_000_000_000),
            age_days: Some(5),
            thumbs_up: Some(10),
            thumbs_down: Some(0),
            guide_facts: vec![],
            extra: Default::default(),
        },
        profile: ProfileDoc {
            id: "test".to_string(),
            name: "Test".to_string(),
            quality_tiers: vec!["2160P".to_string(), "1080P".to_string(), "720P".to_string()],
            archival_quality: Some("2160P".to_string()),
            allow_unknown_quality: false,
            source_allowlist: vec![],
            source_blocklist: vec![],
            video_codec_allowlist: vec![],
            video_codec_blocklist: vec![],
            audio_codec_allowlist: vec![],
            audio_codec_blocklist: vec![],
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: false,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
        },
        context: ContextDoc {
            title_id: Some("tt0000000".to_string()),
            library_name: Some("Movies".to_string()),
            media_type: "movie".to_string(),
            category: "movie".to_string(),
            original_language: Some("eng".to_string()),
            original_country: Some("US".to_string()),
            inferred_original_audio_language: "eng".to_string(),
            tags: vec![],
            has_existing_file: false,
            existing_score: None,
            search_mode: "auto".to_string(),
            runtime_minutes: Some(120),
            is_anime: false,
            is_filler: false,
        },
        builtin_score: BuiltinScoreDoc {
            total: 3200,
            blocked: false,
            codes: vec!["quality_tier_0".to_string(), "source_webdl".to_string()],
        },
        file: Some(FileDoc {
            video_codec: Some("hevc".to_string()),
            video_width: Some(3840),
            video_height: Some(2160),
            video_bitrate_kbps: Some(40000),
            video_bit_depth: Some(10),
            video_hdr_format: Some("HDR10".to_string()),
            dovi_profile: Some(8),
            dovi_bl_compat_id: Some(1),
            video_frame_rate: Some("23.976".to_string()),
            video_profile: Some("Main 10".to_string()),
            audio_codec: Some("eac3".to_string()),
            audio_profile: Some("Dolby Digital Plus + Dolby Atmos".to_string()),
            audio_channels: Some(6),
            audio_bitrate_kbps: Some(640),
            audio_languages: vec!["eng".to_string()],
            audio_streams: vec![AudioStreamDoc {
                codec: Some("eac3".to_string()),
                profile: Some("Dolby Digital Plus + Dolby Atmos".to_string()),
                channels: Some(6),
                language: Some("eng".to_string()),
                name: None,
                bitrate_kbps: Some(640),
            }],
            subtitle_languages: vec!["eng".to_string()],
            subtitle_codecs: vec!["subrip".to_string()],
            subtitle_streams: vec![SubtitleStreamDoc {
                codec: Some("subrip".to_string()),
                language: Some("eng".to_string()),
                name: Some("English".to_string()),
                forced: false,
                default: true,
            }],
            has_multiaudio: false,
            duration_seconds: Some(7200),
            num_chapters: Some(12),
            container_format: Some("matroska".to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn built_in_templates() -> Vec<(String, String)> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("apps")
            .join("scryer-web")
            .join("lib")
            .join("constants")
            .join("rule-templates.ts");
        let source = fs::read_to_string(path).expect("rule templates file should be readable");
        let mut templates = Vec::new();
        let mut cursor = source.as_str();

        while let Some(id_start) = cursor.find("id: \"") {
            cursor = &cursor[id_start + 5..];
            let id_end = cursor
                .find('"')
                .expect("template id should be terminated by a quote");
            let id = cursor[..id_end].to_string();
            cursor = &cursor[id_end..];

            let rego_marker = "regoSource: `";
            let rego_start = cursor
                .find(rego_marker)
                .expect("template should define regoSource");
            cursor = &cursor[rego_start + rego_marker.len()..];
            let rego_end = cursor
                .find("`,")
                .expect("template regoSource should terminate with backtick-comma");
            let rego_source = cursor[..rego_end].to_string();
            templates.push((id, rego_source));
            cursor = &cursor[rego_end + 2..];
        }

        assert!(!templates.is_empty(), "should parse built-in templates");
        templates
    }

    fn built_in_template_source(template_id: &str) -> String {
        built_in_templates()
            .into_iter()
            .find(|(id, _)| id == template_id)
            .map(|(_, rego_source)| rego_source)
            .unwrap_or_else(|| panic!("missing built-in template {template_id}"))
    }

    fn evaluate_template(
        template_id: &str,
        input: UserRuleInput,
        facet: &str,
    ) -> crate::EvalResult {
        let policy_id = format!("builtin_{}", template_id.replace('-', "_"));
        let rego_source =
            crate::rewrite_package_declaration(&built_in_template_source(template_id), &policy_id);
        let engine = crate::UserRulesEngine::build(&[crate::UserPolicy {
            id: policy_id,
            name: template_id.to_string(),
            rego_source,
            origin: crate::PolicyOrigin::User,
            applied_facets: Vec::new(),
        }])
        .expect("template policy should compile");
        let mut evaluator = engine.evaluator();
        evaluator
            .evaluate(&input, facet)
            .expect("template evaluation should succeed")
    }

    #[test]
    fn valid_rule_passes_validation() {
        let source = r#"
            package scryer.rules.user.test_rule
            import rego.v1

            score_entry["bonus"] := 100
        "#;
        let result = validate_user_rule(source, "test_rule").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn runtime_wrapper_validation_accepts_persisted_rule() {
        let source = r#"
            package scryer.rules.user.persisted_rule
            import rego.v1

            score_entry["bonus"] := 100
        "#;

        let result = validate_runtime_wrapper(source, "persisted_rule").unwrap();

        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn runtime_wrapper_validation_rejects_persisted_rule_that_fails_at_runtime() {
        let source = r#"
            package scryer.rules.user.runtime_failure
            import rego.v1

            score_entry["bonus"] := lower(input.release.year)
        "#;

        let result = validate_runtime_wrapper(source, "runtime_failure").unwrap();

        assert!(!result.valid);
        assert!(result.errors[0].contains("runtime error"));
    }

    #[test]
    fn rule_input_contract_copies_are_byte_identical() {
        assert_eq!(
            include_str!("../rule-input-contract.json"),
            include_str!("../../../apps/scryer-web/lib/contracts/rule-input-contract.json"),
            "crates/scryer-rules/rule-input-contract.json and \
             apps/scryer-web/lib/contracts/rule-input-contract.json must stay byte-identical"
        );
    }

    #[test]
    fn maintenance_input_contract_copies_are_byte_identical() {
        assert_eq!(
            include_str!("../maintenance-input-contract.json"),
            include_str!("../../../apps/scryer-web/lib/contracts/maintenance-input-contract.json"),
            "crates/scryer-rules/maintenance-input-contract.json and \
             apps/scryer-web/lib/contracts/maintenance-input-contract.json must stay byte-identical"
        );
    }

    fn validate_maintenance_body(id: &str, body: &str) -> ValidationResult {
        let source = maintenance::rewrite_package_declaration(body, id);
        validate_maintenance_rule(&source, id).expect("validation should not fail outright")
    }

    fn maintenance_policy(id: &str, body: &str) -> maintenance::MaintenancePolicy {
        maintenance::MaintenancePolicy {
            id: id.to_string(),
            name: format!("rule {id}"),
            rego_source: maintenance::rewrite_package_declaration(body, id),
        }
    }

    /// The engine is the second gate: a revision stored before a rule existed
    /// (or one that somehow bypassed validation) must still be refused at load,
    /// so the fact set the engine holds a rule on is never an underestimate.
    fn maintenance_build_error(id: &str, body: &str) -> String {
        maintenance::MaintenanceRulesEngine::build(&[maintenance_policy(id, body)])
            .err()
            .map(|error| error.to_string())
            .unwrap_or_else(|| panic!("engine build should have refused {id}"))
    }

    /// Both gates refuse a source, with the same explanation.
    fn assert_maintenance_rejected_everywhere(id: &str, body: &str, expected: &str) {
        let result = validate_maintenance_body(id, body);
        assert!(!result.valid, "{id} should not validate");
        assert!(
            result.errors.iter().any(|error| error.contains(expected)),
            "{id} validation errors: {:?}",
            result.errors
        );

        let build_error = maintenance_build_error(id, body);
        assert!(
            build_error.contains(expected),
            "{id} build error: {build_error}"
        );
    }

    #[test]
    fn valid_maintenance_rule_passes_validation() {
        let result = validate_maintenance_body(
            "unmonitored_and_stale",
            "match if {\n  \
               not input.facts.monitored\n  \
               input.facts.files[0].quality == \"2160P\"\n\
             }\n\n\
             reasons contains \"unmonitored\"\n",
        );
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    /// The advanced surface stays documented and reachable: envelope fields,
    /// including the per-file detail under `value[]`, still validate.
    #[test]
    fn a_maintenance_rule_may_read_the_observation_envelope_directly() {
        let result = validate_maintenance_body(
            "envelope_reader",
            "match if {\n  \
               input.observations.monitored.status == \"known\"\n  \
               not input.observations.monitored.value\n  \
               input.observations.files.value[0].quality == \"2160P\"\n\
             }\n\n\
             unknown if {\n  \
               input.observations.last_upgraded_at.status == \"unknown\"\n\
             }\n\n\
             reasons contains input.observations.last_upgraded_at.reason\n",
        );
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn maintenance_rule_without_match_is_rejected() {
        let result = validate_maintenance_body(
            "no_match_rule",
            "unknown if {\n  not input.facts.monitored\n}\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("must define a boolean 'match' rule"),
            "{:?}",
            result.errors
        );
    }

    /// The referenced-fact set is what makes an unobservable fact hold a rule,
    /// so a fact name the host cannot read off the source is refused outright.
    #[test]
    fn maintenance_rule_with_dynamic_fact_access_is_rejected() {
        let result = validate_maintenance_body(
            "dynamic_fact",
            "match if {\n  some fact\n  input.facts[fact]\n}\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors.iter().any(|error| {
                error.contains("Unsupported dynamic fact access")
                    && error.contains("input.facts.monitored")
            }),
            "{:?}",
            result.errors
        );
    }

    /// Passing the whole fact object to a builtin sidesteps the referenced-fact
    /// set the same way a computed name would: `object.get(input.facts, "x",
    /// false)` reads fact `x` without ever writing the path, so an unknown `x`
    /// would decide as a plain false instead of holding the rule.
    #[test]
    fn maintenance_rule_reading_the_whole_fact_object_is_rejected() {
        let result = validate_maintenance_body(
            "whole_object",
            "match if {\n  object.get(input.facts, \"monitored\", false)\n}\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors.iter().any(|error| {
                error.contains("Unsupported reference to the whole fact object")
                    && error.contains("input.observations")
            }),
            "{:?}",
            result.errors
        );

        let build_error = maintenance_build_error(
            "whole_object",
            "match if {\n  object.get(input.facts, \"monitored\", false)\n}\n",
        );
        assert!(
            build_error.contains("Unsupported reference to the whole fact object"),
            "{build_error}"
        );
    }

    /// Handing the whole document around defeats the referenced-fact set even
    /// more completely than reading `input.facts` wholesale: every one of these
    /// reads a fact without ever writing a fact path, so the engine's auto-hold
    /// would never fire and an unobservable fact would decide as a plain
    /// absence. Both gates — validation and engine build — refuse them.
    #[test]
    fn maintenance_rule_referencing_the_whole_input_document_is_rejected() {
        const EXPECTED: &str = "Unsupported reference to the whole input document";

        assert_maintenance_rejected_everywhere(
            "input_alias",
            "match if {\n  \
               doc := input\n  \
               doc.facts.watched_by_user_ids\n\
             }\n",
            EXPECTED,
        );

        assert_maintenance_rejected_everywhere(
            "input_object_get",
            "match if {\n  object.get(input, [\"facts\", \"monitored\"], false)\n}\n",
            EXPECTED,
        );

        assert_maintenance_rejected_everywhere(
            "input_as_function_argument",
            "match if {\n  watched(input)\n}\n\n\
             watched(doc) if {\n  doc.facts.last_watched_at\n}\n",
            EXPECTED,
        );

        assert_maintenance_rejected_everywhere(
            "input_walk",
            "match if {\n  \
               some path, value\n  \
               walk(input, [path, value])\n  \
               path == [\"facts\", \"monitored\"]\n  \
               value\n\
             }\n",
            EXPECTED,
        );

        assert_maintenance_rejected_everywhere(
            "input_bare_comparison",
            "match if {\n  input == {}\n}\n",
            EXPECTED,
        );
    }

    /// `with input as {...}` reaches the same place by a different door: the
    /// override target is a bare `input`, so the rule body underneath it reads
    /// facts that never appear in the referenced-fact set.
    #[test]
    fn maintenance_rule_overriding_the_whole_input_document_is_rejected() {
        assert_maintenance_rejected_everywhere(
            "input_with_override",
            "match if {\n  \
               inner with input as {\"facts\": {\"monitored\": false}}\n\
             }\n\n\
             inner if {\n  not input.facts.monitored\n}\n",
            "Unsupported reference to the whole input document",
        );
    }

    /// Over-approximating on purpose: a `with input.facts.<name> as ...`
    /// override still names its fact, so the referenced-fact set keeps counting
    /// it — the rule is held on a fact it only substitutes for, which errs
    /// toward holding rather than deciding.
    #[test]
    fn maintenance_rule_may_override_a_named_fact() {
        let body = "match if {\n  \
                      inner with input.facts.monitored as false\n\
                    }\n\n\
                    inner if {\n  not input.facts.monitored\n}\n";
        let result = validate_maintenance_body("named_fact_override", body);
        assert!(result.valid, "errors: {:?}", result.errors);

        let facts = maintenance_referenced_facts(
            &maintenance::rewrite_package_declaration(body, "named_fact_override"),
            "named_fact_override",
        )
        .expect("named-fact override should resolve a fact set");
        assert!(
            facts.contains("monitored"),
            "override should still count the fact: {facts:?}"
        );
    }

    /// The rejection is exactly a *bare* `input`: the walker emits maximal
    /// paths, so an ordinary fact read must stay untouched at both gates.
    #[test]
    fn maintenance_rule_reading_a_named_fact_is_still_accepted() {
        let body = "match if {\n  not input.facts.monitored\n}\n";
        let result = validate_maintenance_body("named_fact_read", body);
        assert!(result.valid, "errors: {:?}", result.errors);

        maintenance::MaintenanceRulesEngine::build(&[maintenance_policy("named_fact_read", body)])
            .expect("a named fact read should load");
    }

    /// Release rules keep today's behavior: `input` there is not a fact-set
    /// oracle, so a bare reference is no worse than any other broad read.
    #[test]
    fn release_rule_may_reference_input_as_a_whole() {
        let source = r#"
            package scryer.rules.user.bare_input
            import rego.v1

            score_entry["bonus"] := 100 if {
                object.get(input, ["context", "is_anime"], false)
            }
        "#;
        let result = validate_user_rule(source, "bare_input").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn maintenance_rule_importing_part_of_input_is_rejected() {
        let source = maintenance::rewrite_package_declaration(
            "import input.facts\n\nmatch if {\n  facts.monitored\n}\n",
            "imported_facts",
        );
        let result = validate_maintenance_rule(&source, "imported_facts").expect("validates");
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("Unsupported import 'input.facts'"),
            "{:?}",
            result.errors
        );
    }

    /// Every matcher the web offers as a starting point must validate exactly
    /// as written; a template that needs editing before it saves is not a
    /// template. Pinned here rather than parsed out of the gallery so the two
    /// have to be changed together deliberately.
    #[test]
    fn every_pinned_maintenance_template_validates() {
        let templates: [(&str, &str); 12] = [
            (
                "dead-wanted",
                "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.monitored\n\tnot input.facts.has_file\n}\n",
            ),
            (
                "library-aging",
                "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.has_file\n\tnot \"keep\" in input.facts.tags\n}\n",
            ),
            (
                "added-age",
                "package rules\nimport rego.v1\n\nday_ns := (24 * 60 * 60) * 1000000000\n\nmatch if {\n\tage := time.parse_rfc3339_ns(input.evaluation_time) - time.parse_rfc3339_ns(input.facts.added_at)\n\tage > 180 * day_ns\n}\n",
            ),
            (
                "oversized",
                "package rules\nimport rego.v1\n\nmatch if input.facts.total_file_size_bytes > 40000000000\n",
            ),
            (
                "4k-purge",
                "package rules\nimport rego.v1\n\nmatch if {\n\tsome file in input.facts.files\n\tfile.video_height >= 2160\n}\n",
            ),
            (
                "requested-expiry",
                "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.requested\n\tnot \"keep\" in input.facts.tags\n}\n",
            ),
            (
                "departed-requester",
                "package rules\nimport rego.v1\n\nmatch if {\n\t\"departed-user\" in input.facts.requested_by_usernames\n}\n",
            ),
            (
                "system-added",
                "package rules\nimport rego.v1\n\nmatch if {\n\tnot input.facts.added_by_user_id\n\tinput.facts.has_file\n}\n",
            ),
            (
                "watched-by-every-requester",
                "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.requested\n\tinput.facts.watched_by_all_requesters\n}\n",
            ),
            (
                "no-profile",
                "package rules\nimport rego.v1\n\nmatch if not input.facts.quality_profile_id\n",
            ),
            (
                "tagged-for-removal",
                "package rules\nimport rego.v1\n\nmatch if {\n\t\"remove\" in input.facts.tags\n}\n",
            ),
            (
                "flag-for-review",
                "package rules\nimport rego.v1\n\nday_ns := (24 * 60 * 60) * 1000000000\n\nmatch if {\n\tinput.facts.has_file\n\tage := time.parse_rfc3339_ns(input.evaluation_time) - time.parse_rfc3339_ns(input.facts.first_imported_at)\n\tage > 365 * day_ns\n}\n",
            ),
        ];

        for (index, (template_id, rego_source)) in templates.into_iter().enumerate() {
            let rule_id = format!("maintenance_template_{index}");
            let rewritten = maintenance::rewrite_package_declaration(rego_source, &rule_id);
            let result = validate_maintenance_rule(&rewritten, &rule_id).expect("validates");
            assert!(result.valid, "{template_id}: {:?}", result.errors);
        }
    }

    #[test]
    fn maintenance_rule_with_non_boolean_match_is_rejected() {
        let result = validate_maintenance_body("numeric_match", "match := 42\n");
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("'match' must be a boolean"),
            "{:?}",
            result.errors
        );
    }

    #[test]
    fn maintenance_rule_with_unknown_input_path_is_rejected() {
        let result = validate_maintenance_body(
            "watch_count",
            "match if {\n  input.facts.watch_count == 0\n}\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("Unknown rule input path 'input.facts.watch_count'"),
            "{:?}",
            result.errors
        );
    }

    /// The `input.release.extra.<key>` escape hatch is release-only: maintenance
    /// facts are all catalogued, so an uncatalogued path is always a typo.
    #[test]
    fn maintenance_rule_cannot_use_release_extra_paths() {
        let result = validate_maintenance_body(
            "extra_path",
            "match if {\n  input.release.extra.anything == \"x\"\n}\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("Unknown rule input path 'input.release.extra.anything'"),
            "{:?}",
            result.errors
        );
    }

    #[test]
    fn maintenance_rule_with_wrong_package_is_rejected() {
        let source = maintenance::rewrite_package_declaration("match := true\n", "actual_id");
        let result = validate_maintenance_rule(&source, "expected_id").unwrap();
        assert!(!result.valid);
        assert!(result.errors[0].contains("package declaration"));
    }

    /// Opt-in managed packs may veto, so the builtin is accepted
    /// in managed source. The bound that still applies is evaluation-time.
    #[test]
    fn managed_rule_accepts_block_score_builtin() {
        let source = r#"
            package scryer.rules.user.managed_rule
            import rego.v1
            score_entry["blocked"] := scryer.block_score()
        "#;

        let result = validate_managed_rule(source, "managed_rule").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn wrong_package_name_rejected() {
        let source = r#"
            package scryer.rules.user.wrong_name
            import rego.v1

            score_entry["bonus"] := 100
        "#;
        let result = validate_user_rule(source, "expected_name").unwrap();
        assert!(!result.valid);
        assert!(result.errors[0].contains("package declaration"));
    }

    #[test]
    fn syntax_error_rejected() {
        let source = r#"
            package scryer.rules.user.bad_syntax
            this is not valid rego at all
        "#;
        let result = validate_user_rule(source, "bad_syntax").unwrap();
        assert!(!result.valid);
        assert!(result.errors[0].contains("compilation error"));
    }

    #[test]
    fn conditional_rule_passes_when_condition_not_met() {
        let source = r#"
            package scryer.rules.user.conditional
            import rego.v1

            score_entry["only_anime"] := 100 if {
                input.context.is_anime
            }
        "#;
        let result = validate_user_rule(source, "conditional").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn rule_using_builtin_passes() {
        let source = r#"
            package scryer.rules.user.with_builtin
            import rego.v1

            score_entry["size_block"] := scryer.block_score() if {
                scryer.size_gib(input.release.size_bytes) > 100
            }
        "#;
        let result = validate_user_rule(source, "with_builtin").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn float_output_rejected() {
        let source = r#"
            package scryer.rules.user.float_rule
            import rego.v1

            score_entry["bad"] := 3.14
        "#;
        let result = validate_user_rule(source, "float_rule").unwrap();
        assert!(!result.valid);
        assert!(result.errors[0].contains("float"));
    }

    #[test]
    fn unknown_release_field_rejected() {
        let source = r#"
            package scryer.rules.user.unknown_release_field
            import rego.v1

            score_entry["bad"] := 100 if {
                input.release.password_protected != null
            }
        "#;
        let result = validate_user_rule(source, "unknown_release_field").unwrap();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("input.release.password_protected"))
        );
        assert!(result.errors.iter().any(|error| {
            error.contains("input.release.is_password_protected")
                && error.contains("Rules Context Reference")
        }));
    }

    #[test]
    fn unknown_context_field_rejected() {
        let source = r#"
            package scryer.rules.user.unknown_context_field
            import rego.v1

            score_entry["bad"] := 100 if {
                input.context.missing_flag
            }
        "#;
        let result = validate_user_rule(source, "unknown_context_field").unwrap();
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("input.context.missing_flag"))
        );
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("Rules Context Reference"))
        );
    }

    #[test]
    fn release_extra_dot_access_is_allowed() {
        let source = r#"
            package scryer.rules.user.release_extra_field
            import rego.v1

            score_entry["bonus"] := 100 if {
                input.release.extra.freeleech == true
            }
        "#;
        let result = validate_user_rule(source, "release_extra_field").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn array_variable_index_access_is_allowed() {
        let source = r#"
            package scryer.rules.user.array_variable_index
            import rego.v1

            score_entry["eng_bonus"] := 100 if {
                some i
                input.file.audio_languages[i] == "eng"
            }
        "#;
        let result = validate_user_rule(source, "array_variable_index").unwrap();
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn unsupported_dynamic_non_extra_path_rejected_with_guidance() {
        let source = r#"
            package scryer.rules.user.dynamic_context_lookup
            import rego.v1

            score_entry["bad"] := 100 if {
                some key
                input.context[key]
            }
        "#;
        let result = validate_user_rule(source, "dynamic_context_lookup").unwrap();
        assert!(!result.valid);
        assert!(result.errors.iter().any(|error| {
            error.contains("Unsupported dynamic rule input path")
                && error.contains("documented array indexing")
                && error.contains("input.release.extra.<key>")
        }));
    }

    #[test]
    fn all_built_in_templates_validate_after_rewrite() {
        for (index, (template_id, rego_source)) in built_in_templates().into_iter().enumerate() {
            let rule_id = format!("builtin_{index}");
            let rewritten = crate::rewrite_package_declaration(&rego_source, &rule_id);
            let result = validate_user_rule(&rewritten, &rule_id).unwrap();
            assert!(result.valid, "{template_id}: {:?}", result.errors);
        }
    }

    #[test]
    fn canonical_quality_templates_fire() {
        let webdl_result = evaluate_template("prefer-web-dl", synthetic_test_input(), "movie");
        assert!(
            webdl_result
                .entries
                .iter()
                .any(|entry| entry.code == "prefer_webdl" && entry.delta == 100),
            "prefer-web-dl should match canonical WEB-DL input"
        );

        let x265_result = evaluate_template("prefer-x265", synthetic_test_input(), "movie");
        assert!(
            x265_result
                .entries
                .iter()
                .any(|entry| entry.code == "x265_bonus" && entry.delta == 100),
            "prefer-x265 should match canonical H.265 input"
        );

        let mut x264_input = synthetic_test_input();
        x264_input.release.video_codec = Some("H.264".to_string());
        let x264_result = evaluate_template("penalize-x264-4k", x264_input, "movie");
        assert!(
            x264_result
                .entries
                .iter()
                .any(|entry| entry.code == "x264_4k_penalty" && entry.delta == -200),
            "penalize-x264-4k should match canonical 2160P H.264 input"
        );
    }

    #[test]
    fn group_templates_noop_when_release_group_is_missing() {
        for template_id in [
            "anime-group-preference",
            "block-mini-encodes",
            "block-low-quality-groups",
        ] {
            let mut input = synthetic_test_input();
            input.release.release_group = None;
            let result = evaluate_template(template_id, input, "anime");
            assert!(
                result.errors.is_empty(),
                "{template_id} should not raise runtime errors when release_group is null"
            );
            assert!(
                result.entries.is_empty(),
                "{template_id} should no-op when release_group is null"
            );
        }
    }

    #[test]
    fn password_protected_template_matches_injected_signal() {
        let mut input = synthetic_test_input();
        input.release.is_password_protected = Some(true);
        let result = evaluate_template("block-password-protected", input, "movie");
        assert!(
            result
                .entries
                .iter()
                .any(|entry| entry.code == "password_protected"),
            "password-protected template should block when the signal is injected"
        );
    }
}
