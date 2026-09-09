use crate::maintenance::{self, USER_PACKAGE_PREFIX as MAINTENANCE_USER_PACKAGE_PREFIX};
use crate::request::{
    self, PERSON_TARGETED_REQUEST_ROOT, USER_PACKAGE_PREFIX as REQUEST_USER_PACKAGE_PREFIX,
};
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
    collections::{BTreeSet, HashMap, HashSet},
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

/// One policy family's input contract, and the catalog lazily derived from it.
///
/// Adding a family is adding one `static` here: the contract JSON is the only
/// family-specific thing the path walker needs, and the mirrored copy under
/// `apps/scryer-web/lib/contracts/` is what the Rules Context Reference renders
/// from.
struct FamilyContract {
    json: &'static str,
    name: &'static str,
    catalog: OnceLock<RuleInputCatalog>,
}

impl FamilyContract {
    const fn new(json: &'static str, name: &'static str) -> Self {
        Self {
            json,
            name,
            catalog: OnceLock::new(),
        }
    }

    fn catalog(&'static self) -> &'static RuleInputCatalog {
        self.catalog
            .get_or_init(|| build_input_catalog(self.json, self.name))
    }
}

static RELEASE_CONTRACT: FamilyContract = FamilyContract::new(
    include_str!("../rule-input-contract.json"),
    "rule-input-contract.json",
);

static MAINTENANCE_CONTRACT: FamilyContract = FamilyContract::new(
    include_str!("../maintenance-input-contract.json"),
    "maintenance-input-contract.json",
);

static REQUEST_CONTRACT: FamilyContract = FamilyContract::new(
    include_str!("../request-input-contract.json"),
    "request-input-contract.json",
);

/// The request family's input contract exactly as shipped.
///
/// The Rules Context Reference renders this document. The web app has a
/// byte-identical mirror under `apps/scryer-web/lib/contracts/`, but the API
/// serves the crate's copy so a client that fetched the reference and a server
/// that validated a matcher against it can never be reading different versions
/// of the same contract.
pub fn request_input_contract_json() -> &'static str {
    REQUEST_CONTRACT.json
}

/// Everything the input-path walker needs to judge one policy family's paths.
///
/// The release family alone allows `input.release.extra.<key>`, whose keys come
/// from indexer-supplied attributes and so cannot be catalogued.
#[derive(Debug, Clone, Copy)]
struct InputPathContext {
    /// Family label used in the diagnostics that name the family.
    family: &'static str,
    catalog: &'static RuleInputCatalog,
    allow_release_extra: bool,
    /// Families with observation envelopes only: every `input.facts.<name>`
    /// must name its fact with a literal, because the engine reads that set to
    /// decide whether the subject is even knowable enough to consult the rule.
    static_facts_only: bool,
}

impl InputPathContext {
    fn release() -> Self {
        Self {
            family: "release",
            catalog: RELEASE_CONTRACT.catalog(),
            allow_release_extra: true,
            static_facts_only: false,
        }
    }

    fn maintenance() -> Self {
        Self {
            family: "maintenance",
            catalog: MAINTENANCE_CONTRACT.catalog(),
            allow_release_extra: false,
            static_facts_only: true,
        }
    }

    fn request() -> Self {
        Self {
            family: "request",
            catalog: REQUEST_CONTRACT.catalog(),
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
    fn fact_name(&self) -> Option<&str> {
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

    /// The person-targeted path this reference reads, when it reads one.
    ///
    /// For the request family every field of `input.requester` is about one
    /// named person, so the unit is the subtree rather than a list of facts:
    /// `input.requester.username` reports itself, `input.requester.app_permissions[0]`
    /// reports `input.requester.app_permissions`, and a bare `input.requester`
    /// reports the root — reading the whole document is at least as targeted as
    /// reading one field of it.
    fn person_targeted_path(&self) -> Option<String> {
        // The two segments of `PERSON_TARGETED_REQUEST_ROOT`, which the test
        // below pins so this match and that constant cannot drift.
        match self.components.as_slice() {
            [
                InputPathComponent::Field(input),
                InputPathComponent::Field(requester),
            ] if input == "input" && requester == "requester" => {
                Some(PERSON_TARGETED_REQUEST_ROOT.to_string())
            }
            [
                InputPathComponent::Field(input),
                InputPathComponent::Field(requester),
                InputPathComponent::Field(field),
                ..,
            ] if input == "input" && requester == "requester" => {
                Some(format!("{PERSON_TARGETED_REQUEST_ROOT}.{field}"))
            }
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum StaticReferenceComponent {
    Field(String),
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StaticReference {
    components: Vec<StaticReferenceComponent>,
}

impl StaticReference {
    fn field(&self, index: usize) -> Option<&str> {
        match self.components.get(index)? {
            StaticReferenceComponent::Field(field) => Some(field),
            StaticReferenceComponent::Dynamic => None,
        }
    }

    fn starts_with(&self, fields: &[&str]) -> bool {
        fields
            .iter()
            .enumerate()
            .all(|(index, field)| self.field(index) == Some(*field))
    }
}

fn static_reference(expr: &Expr) -> Option<StaticReference> {
    match expr {
        Expr::Var { span, .. } => Some(StaticReference {
            components: vec![StaticReferenceComponent::Field(span.text().to_string())],
        }),
        Expr::RefDot { refr, field, .. } => {
            let mut reference = static_reference(refr)?;
            reference.components.push(StaticReferenceComponent::Field(
                field.1.as_string().ok()?.to_string(),
            ));
            Some(reference)
        }
        Expr::RefBrack { refr, index, .. } => {
            let mut reference = static_reference(refr)?;
            let component = match index.as_ref() {
                Expr::String { value, .. } | Expr::RawString { value, .. } => {
                    StaticReferenceComponent::Field(value.as_string().ok()?.to_string())
                }
                _ => StaticReferenceComponent::Dynamic,
            };
            reference.components.push(component);
            Some(reference)
        }
        _ => None,
    }
}

fn rule_reference_aliases(module: &Module, rule: &Rule) -> HashMap<String, StaticReference> {
    let mut aliases = module
        .imports
        .iter()
        .filter_map(|import| {
            let reference = static_reference(&import.refr)?;
            let alias = import
                .r#as
                .as_ref()
                .map(|span| span.text().trim().to_string())
                .or_else(|| {
                    reference
                        .field(reference.components.len().checked_sub(1)?)
                        .map(str::to_string)
                })?;
            Some((alias, reference))
        })
        .collect::<HashMap<_, _>>();
    // Rego variables are immutable within a rule. A direct assignment to a
    // static reference is therefore a useful alias without looking at source
    // text; unresolved or dynamic assignments are deliberately ignored.
    visit_rule_expr_nodes(rule, |expr| {
        let Expr::AssignExpr { lhs, rhs, .. } = expr else {
            return;
        };
        let Expr::Var { span, .. } = lhs.as_ref() else {
            return;
        };
        if let Some(reference) =
            static_reference(rhs).or_else(|| object_get_reference(rhs, &aliases))
        {
            let reference = resolve_reference(reference, &aliases);
            aliases.insert(span.text().to_string(), reference);
        }
    });
    aliases
}

fn resolve_reference(
    mut reference: StaticReference,
    aliases: &HashMap<String, StaticReference>,
) -> StaticReference {
    let mut seen = HashSet::new();
    while let Some(root) = reference.field(0).map(str::to_string) {
        let Some(alias) = aliases.get(&root) else {
            break;
        };
        if !seen.insert(root) {
            break;
        }
        let mut components = alias.components.clone();
        components.extend(reference.components.drain(1..));
        reference.components = components;
    }
    reference
}

fn visit_expr_nodes(expr: &Expr, sink: &mut dyn FnMut(&Expr)) {
    sink(expr);
    match expr {
        Expr::Array { items, .. } | Expr::Set { items, .. } => {
            for item in items {
                visit_expr_nodes(item, sink);
            }
        }
        Expr::Object { fields, .. } => {
            for (_, key, value) in fields {
                visit_expr_nodes(key, sink);
                visit_expr_nodes(value, sink);
            }
        }
        Expr::ArrayCompr { term, query, .. } | Expr::SetCompr { term, query, .. } => {
            visit_expr_nodes(term, sink);
            visit_query_expr_nodes(query, sink);
        }
        Expr::ObjectCompr {
            key, value, query, ..
        } => {
            visit_expr_nodes(key, sink);
            visit_expr_nodes(value, sink);
            visit_query_expr_nodes(query, sink);
        }
        Expr::Call { fcn, params, .. } => {
            visit_expr_nodes(fcn, sink);
            // A resolved object.get is one reference. Walking its base again
            // would mistake a bounded field read for a whole-document read.
            let skip = if object_get_reference(expr, &HashMap::new()).is_some() {
                1
            } else {
                0
            };
            for param in params.iter().skip(skip) {
                visit_expr_nodes(param, sink);
            }
        }
        Expr::UnaryExpr { expr, .. } => visit_expr_nodes(expr, sink),
        Expr::RefDot { refr, .. } => {
            if static_reference(refr).is_none() {
                visit_expr_nodes(refr, sink);
            }
        }
        Expr::RefBrack { refr, index, .. } => {
            if static_reference(refr).is_none() {
                visit_expr_nodes(refr, sink);
            }
            visit_expr_nodes(index, sink);
        }
        Expr::BinExpr { lhs, rhs, .. }
        | Expr::BoolExpr { lhs, rhs, .. }
        | Expr::ArithExpr { lhs, rhs, .. }
        | Expr::AssignExpr { lhs, rhs, .. }
        | Expr::OrExpr { lhs, rhs, .. } => {
            visit_expr_nodes(lhs, sink);
            visit_expr_nodes(rhs, sink);
        }
        Expr::Membership {
            key,
            value,
            collection,
            ..
        } => {
            if let Some(key) = key {
                visit_expr_nodes(key, sink);
            }
            visit_expr_nodes(value, sink);
            visit_expr_nodes(collection, sink);
        }
        Expr::String { .. }
        | Expr::RawString { .. }
        | Expr::Number { .. }
        | Expr::Bool { .. }
        | Expr::Null { .. }
        | Expr::Var { .. } => {}
    }
}

fn visit_query_expr_nodes(query: &Query, sink: &mut dyn FnMut(&Expr)) {
    for statement in &query.stmts {
        match &statement.literal {
            Literal::SomeVars { .. } => {}
            Literal::SomeIn {
                key,
                value,
                collection,
                ..
            } => {
                if let Some(key) = key {
                    visit_expr_nodes(key, sink);
                }
                visit_expr_nodes(value, sink);
                visit_expr_nodes(collection, sink);
            }
            Literal::Expr { expr, .. } | Literal::NotExpr { expr, .. } => {
                visit_expr_nodes(expr, sink)
            }
            Literal::Every { domain, query, .. } => {
                visit_expr_nodes(domain, sink);
                visit_query_expr_nodes(query, sink);
            }
        }
        for with_mod in &statement.with_mods {
            visit_expr_nodes(&with_mod.refr, sink);
            visit_expr_nodes(&with_mod.r#as, sink);
        }
    }
}

fn visit_rule_expr_nodes(rule: &Rule, mut sink: impl FnMut(&Expr)) {
    match rule {
        Rule::Spec { head, bodies, .. } => {
            match head {
                RuleHead::Compr { refr, assign, .. } => {
                    visit_expr_nodes(refr, &mut sink);
                    if let Some(assign) = assign {
                        visit_expr_nodes(&assign.value, &mut sink);
                    }
                }
                RuleHead::Set { refr, key, .. } => {
                    visit_expr_nodes(refr, &mut sink);
                    if let Some(key) = key {
                        visit_expr_nodes(key, &mut sink);
                    }
                }
                RuleHead::Func {
                    refr, args, assign, ..
                } => {
                    visit_expr_nodes(refr, &mut sink);
                    for arg in args {
                        visit_expr_nodes(arg, &mut sink);
                    }
                    if let Some(assign) = assign {
                        visit_expr_nodes(&assign.value, &mut sink);
                    }
                }
            }
            for body in bodies {
                if let Some(assign) = &body.assign {
                    visit_expr_nodes(&assign.value, &mut sink);
                }
                visit_query_expr_nodes(&body.query, &mut sink);
            }
        }
        Rule::Default {
            refr, args, value, ..
        } => {
            visit_expr_nodes(refr, &mut sink);
            for arg in args {
                visit_expr_nodes(arg, &mut sink);
            }
            visit_expr_nodes(value, &mut sink);
        }
    }
}

fn visit_module_references(module: &Module, mut sink: impl FnMut(StaticReference)) {
    for rule in &module.policy {
        let aliases = rule_reference_aliases(module, rule);
        visit_rule_expr_nodes(rule, |expr| {
            if let Some(reference) = static_reference(expr)
                .map(|reference| resolve_reference(reference, &aliases))
                .or_else(|| object_get_reference(expr, &aliases))
            {
                sink(reference);
            }
        });
    }
}

fn object_get_reference(
    expr: &Expr,
    aliases: &HashMap<String, StaticReference>,
) -> Option<StaticReference> {
    let Expr::Call { fcn, params, .. } = expr else {
        return None;
    };
    let function = static_reference(fcn)?;
    if !function.starts_with(&["object", "get"]) || params.len() < 2 {
        return None;
    }
    let base =
        static_reference(&params[0]).or_else(|| object_get_reference(&params[0], aliases))?;
    let mut reference = resolve_reference(base, aliases);
    if let Expr::Array { items, .. } = params[1].as_ref() {
        for item in items {
            reference.components.push(match item.as_ref() {
                Expr::String { value, .. } | Expr::RawString { value, .. } => {
                    StaticReferenceComponent::Field(value.as_string().ok()?.to_string())
                }
                _ => StaticReferenceComponent::Dynamic,
            });
        }
        return Some(reference);
    }
    let key = match params[1].as_ref() {
        Expr::String { value, .. } | Expr::RawString { value, .. } => value.as_string().ok()?,
        _ => {
            reference.components.push(StaticReferenceComponent::Dynamic);
            return Some(reference);
        }
    };
    reference
        .components
        .push(StaticReferenceComponent::Field(key.to_string()));
    Some(reference)
}

/// Finds statically resolvable references to release-input fields removed from
/// the current host contract. Dynamic object keys are intentionally ignored:
/// a retained source is only classified when the AST proves the retired field.
pub fn retired_release_input_fields(rego_source: &str) -> Result<Vec<String>, String> {
    let module = parse_module(rego_source, "scryer.rules.retired_input")?;
    let mut retired = BTreeSet::new();
    for import in &module.imports {
        if static_reference(&import.refr)
            .is_some_and(|reference| reference.starts_with(&["input", "release", "guide_facts"]))
        {
            retired.insert("input.release.guide_facts".to_string());
        }
    }
    visit_module_references(&module, |reference| {
        if reference.starts_with(&["input", "release", "guide_facts"]) {
            retired.insert("input.release.guide_facts".to_string());
        }
    });
    Ok(retired.into_iter().collect())
}

/// Rejects baseline rules that depend on a score produced later, or on an
/// additional user rule. Dynamic reads of either namespace are rejected because
/// the dependency cannot be proven from the AST.
pub fn validate_baseline_dependencies(
    rego_source: &str,
    additional_rule_ids: &[String],
) -> Result<(), String> {
    let module = parse_module(rego_source, "scryer.rules.baseline_dependencies")?;
    let additional_rule_ids = additional_rule_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut errors = BTreeSet::new();
    visit_module_references(&module, |reference| {
        if reference.starts_with(&["input", "builtin_score"]) {
            errors.insert("baseline rules cannot reference input.builtin_score".to_string());
        }
        if reference.starts_with(&["input"])
            && matches!(
                reference.components.get(1),
                Some(StaticReferenceComponent::Dynamic) | None
            )
        {
            errors.insert(
                "baseline rules cannot dynamically read input because builtin_score dependency cannot be proven"
                    .to_string(),
            );
        }
        let namespace = ["data", "scryer", "rules", "user"];
        if reference.field(0) == Some("data")
            && (reference.components.len() < namespace.len()
                || reference
                    .components
                    .iter()
                    .take(namespace.len())
                    .any(|component| matches!(component, StaticReferenceComponent::Dynamic)))
            && reference
                .components
                .iter()
                .take(namespace.len())
                .enumerate()
                .all(|(index, component)| {
                    matches!(component, StaticReferenceComponent::Dynamic)
                        || reference.field(index) == Some(namespace[index])
                })
        {
            errors.insert("baseline rules cannot read the entire rule namespace".to_string());
        }
        if reference.starts_with(&["data", "scryer", "rules", "user"]) {
            match reference.components.get(4) {
                Some(StaticReferenceComponent::Field(id))
                    if additional_rule_ids.contains(id.as_str()) =>
                {
                    errors.insert(format!(
                        "baseline rules cannot depend on additional rule '{id}'"
                    ));
                }
                Some(StaticReferenceComponent::Dynamic) | None => {
                    errors.insert(
                        "baseline rules cannot dynamically read data.scryer.rules.user because additional-rule dependency cannot be proven"
                            .to_string(),
                    );
                }
                _ => {}
            }
        }
    });
    match errors.into_iter().next() {
        Some(error) => Err(error),
        None => Ok(()),
    }
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
fn module_referenced_facts(
    module: &Module,
    ctx: InputPathContext,
) -> Result<BTreeSet<String>, String> {
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
        } else if let Some(name) = path.fact_name() {
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
    fact_references(rego_source, policy_path, InputPathContext::maintenance())
}

pub(crate) fn maintenance_fact_references_from_module(
    module: &Module,
) -> Result<BTreeSet<String>, String> {
    module_fact_references(module, InputPathContext::maintenance())
}

/// The same work for any family whose facts arrive in observation envelopes.
fn fact_references(
    rego_source: &str,
    policy_path: &str,
    ctx: InputPathContext,
) -> Result<BTreeSet<String>, String> {
    let module = parse_module(rego_source, policy_path)?;
    module_fact_references(&module, ctx)
}

fn module_fact_references(
    module: &Module,
    ctx: InputPathContext,
) -> Result<BTreeSet<String>, String> {
    if let Some(error) = input_import_error(module, ctx.family) {
        return Err(error);
    }
    module_referenced_facts(module, ctx)
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

/// Facts a request rule reads, plus the parse work needed to find them.
///
/// The request family holds a rule on unobservable facts exactly as maintenance
/// does, so it needs the same statically resolvable set and fails for the same
/// sources: one that will not parse, imports part of `input`, hands the document
/// around whole, or selects a fact by a computed name.
pub(crate) fn request_fact_references(
    rego_source: &str,
    policy_path: &str,
) -> Result<BTreeSet<String>, String> {
    fact_references(rego_source, policy_path, InputPathContext::request())
}

pub(crate) fn request_fact_references_from_module(
    module: &Module,
) -> Result<BTreeSet<String>, String> {
    module_fact_references(module, InputPathContext::request())
}

/// The `input.facts.<name>` facts a request rule reads, for callers outside this
/// crate that must decide something about a rule *before* it runs.
///
/// The same static set the engine holds rules on, exposed once so an
/// authorization check and the evaluator can never disagree about what a rule
/// reads. A caller that cannot get an answer here must reject the rule rather
/// than assume it reads nothing.
pub fn request_referenced_facts(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<BTreeSet<String>, String> {
    request_fact_references(rego_source, &request::user_policy_path(rule_set_id))
}

/// Every `input.requester.*` path a request rule reads, deduplicated, in the
/// order the source reads them.
///
/// Authoring or previewing a rule that reads any of these is a way of asking the
/// instance about one named person, so the application gates it on
/// permission-management authority — the request-family counterpart of
/// [`crate::maintenance::PERSON_TARGETED_MAINTENANCE_FACTS`]. An empty result
/// means the rule decides on content and the draft alone.
///
/// Fails for the same sources the engine would refuse to load: a rule whose
/// input references cannot be read off its source has no resolvable list here
/// either, and a caller must reject it rather than treat it as person-free.
pub fn request_person_targeted_paths(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<Vec<String>, String> {
    let policy_path = request::user_policy_path(rule_set_id);
    let module = parse_module(rego_source, &policy_path)?;
    let ctx = InputPathContext::request();
    if let Some(error) = input_import_error(&module, ctx.family) {
        return Err(error);
    }

    // Same refusals as the fact set: a rule that reaches the requester document
    // through a bare `input` would read person data without ever writing a
    // person path.
    let mut paths: Vec<String> = Vec::new();
    let mut error: Option<String> = None;
    walk_module_input_paths(&module, ctx, |path| {
        if error.is_some() {
            return;
        }
        if path.references_input_wholesale() {
            error = Some(whole_input_document_message(&path.display));
        } else if let Some(person_path) = path.person_targeted_path()
            && !paths.contains(&person_path)
        {
            paths.push(person_path);
        }
    });

    match error {
        Some(error) => Err(error),
        None => Ok(paths),
    }
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
fn input_import_error(module: &Module, family: &str) -> Option<String> {
    module.imports.iter().find_map(|import| {
        (rule_head_name(&import.refr) == Some("input")).then(|| {
            format!(
                "Unsupported import '{}' in a {family} rule. Reference facts by their full path \
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
    validate_user_rule_with_limits(rego_source, rule_set_id, &RuntimeLimits::release_defaults())
}

/// [`validate_user_rule`] with caller-supplied runtime limits.
///
/// The dry run evaluates the rule under the same host-enforced budget the
/// release-scoring engine applies, so a rule that validates here also runs in
/// production. Callers that exercise the validator outside a release build
/// — corpus tests over hundreds of translated formats in an unoptimised
/// interpreter — pass a wider evaluation budget so wall-clock noise does not
/// masquerade as a rejected rule.
pub fn validate_user_rule_with_limits(
    rego_source: &str,
    rule_set_id: &str,
    limits: &RuntimeLimits,
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
    let mut engine = runtime::configured_engine(limits);

    let policy_path = format!("user/{rule_set_id}.rego");
    if let Err(e) = engine.add_policy(policy_path.clone(), rego_source.to_string()) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }
    let policy_module_index = engine.get_modules().len() - 1;
    if let Err(e) = engine.add_policy(
        score_entry_wrapper_policy_path(rule_set_id),
        score_entry_wrapper_source(rule_set_id),
    ) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    let input_path_errors = module_input_path_errors(
        &engine.get_modules()[policy_module_index],
        InputPathContext::release(),
    );
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
    let policy_module_index = engine.get_modules().len() - 1;
    if let Err(e) = engine.add_policy(
        maintenance::decision_wrapper_policy_path(rule_set_id),
        maintenance::decision_wrapper_source(rule_set_id),
    ) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    let module = &engine.get_modules()[policy_module_index];

    if let Some(error) = input_import_error(module, "maintenance") {
        return Ok(ValidationResult::invalid(error));
    }

    let input_path_errors = module_input_path_errors(module, InputPathContext::maintenance());
    if !input_path_errors.is_empty() {
        return Ok(ValidationResult {
            valid: false,
            errors: input_path_errors,
        });
    }

    if !module_defines_rule(module, "match") {
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

/// The heads a request rule may write. At least one of the first four has to be
/// defined or the rule can never contribute anything to a decision.
const REQUEST_VOTE_HEADS: [&str; 4] = ["approve", "deny", "manual", "tags"];

/// Validate a user-authored request rule without persisting it.
///
/// The caller is expected to have already called
/// [`crate::request::rewrite_package_declaration`] on the source.
///
/// Checks:
/// 1. Package declaration matches `scryer.request.user.<rule_set_id>`.
/// 2. Source and the generated decision wrapper both compile.
/// 3. No import pulls part of `input` into scope, `input` is never referenced as
///    a whole value, and every fact is selected by a literal name. All three keep
///    the referenced-fact set statically resolvable, which is what lets the
///    engine hold a rule whose facts it cannot observe.
/// 4. Every `input.*` reference resolves in the request catalog.
/// 5. The source defines at least one of `approve`, `deny`, `manual`, or `tags`.
///    The wrapper defaults every head, so a rule defining none of them — one
///    that only writes `reasons`, say — would compile, load, and abstain
///    forever; this is the only place that catches it.
/// 6. A dry run against synthetic input yields a well-formed decision, which is
///    also what proves the rule's `reasons` and `tags` are inside their bounds.
pub fn validate_request_rule(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<ValidationResult, RulesError> {
    let expected_pkg = format!("package {REQUEST_USER_PACKAGE_PREFIX}.{rule_set_id}");
    if !rego_source.lines().any(|line| line.trim() == expected_pkg) {
        return Ok(ValidationResult::invalid(format!(
            "package declaration must be: {expected_pkg}"
        )));
    }

    let mut engine = runtime::configured_engine(&RuntimeLimits::request_defaults());

    let policy_path = request::user_policy_path(rule_set_id);
    if let Err(e) = engine.add_policy(policy_path.clone(), rego_source.to_string()) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }
    let policy_module_index = engine.get_modules().len() - 1;
    if let Err(e) = engine.add_policy(
        request::decision_wrapper_policy_path(rule_set_id),
        request::decision_wrapper_source(rule_set_id),
    ) {
        return Ok(ValidationResult::invalid(format!("compilation error: {e}")));
    }

    let module = &engine.get_modules()[policy_module_index];

    if let Some(error) = input_import_error(module, "request") {
        return Ok(ValidationResult::invalid(error));
    }

    let input_path_errors = module_input_path_errors(module, InputPathContext::request());
    if !input_path_errors.is_empty() {
        return Ok(ValidationResult {
            valid: false,
            errors: input_path_errors,
        });
    }

    if !REQUEST_VOTE_HEADS
        .iter()
        .any(|head| module_defines_rule(module, head))
    {
        return Ok(ValidationResult::invalid(
            "request rule can never vote: define at least one of 'approve', 'deny', 'manual', \
             or 'tags', for example: approve if { ... }",
        ));
    }

    let input_value = serde_json::to_value(request::synthetic_request_input())
        .map_err(RulesError::Serialization)?;
    engine.set_input(input_value.into());

    match engine.eval_rule(request::decision_wrapper_rule_path(rule_set_id)) {
        Ok(value) => match request::decode_decision(&value) {
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
            normalized_tokens: vec![],
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
            has_hdr_fallback: false,
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
            scoring_persona: "balanced".to_string(),
            scoring_overrides: Default::default(),
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
            coverage_total_runtime_minutes: Some(120),
            coverage_member_runtime_minutes: Some(120),
            coverage_member_count: Some(1),
            is_anime: false,
            is_filler: false,
        },
        builtin_score: BuiltinScoreDoc {
            total: 3200,
            blocked: false,
            codes: vec!["quality_tier_0".to_string(), "source_webdl".to_string()],
        },
        file: Some(FileDoc {
            details: Default::default(),
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

    fn padded_rule_source(rule_set_id: &str, comment_count: usize, comment_width: usize) -> String {
        assert!(comment_width > 0);
        let comment = format!("#{}\n", "x".repeat(comment_width - 1));
        format!(
            "package scryer.rules.user.{rule_set_id}\n\
             import rego.v1\n\
             {}\
             score_entry[\"bonus\"] := 100\n",
            comment.repeat(comment_count),
        )
    }

    fn test_policy(rule_set_id: &str, rego_source: &str) -> crate::UserPolicy {
        crate::UserPolicy {
            id: rule_set_id.to_string(),
            name: "Test Rule".to_string(),
            rego_source: rego_source.to_string(),
            origin: crate::PolicyOrigin::User,
            applied_facets: Vec::new(),
        }
    }

    fn assert_validation_and_runtime_reject(rule_set_id: &str, source: &str) {
        let edit_validation = validate_user_rule(source, rule_set_id).unwrap();
        assert!(
            !edit_validation.valid,
            "editor validation unexpectedly accepted the source"
        );

        let wrapper_validation = validate_runtime_wrapper(source, rule_set_id).unwrap();
        assert!(
            !wrapper_validation.valid,
            "runtime-wrapper validation unexpectedly accepted the source"
        );

        assert!(
            crate::UserRulesEngine::build(&[test_policy(rule_set_id, source)]).is_err(),
            "runtime unexpectedly accepted the source"
        );
    }

    #[test]
    fn release_policy_limits_accept_sources_above_regorus_defaults() {
        let limits = RuntimeLimits::release_defaults();
        let rule_set_id = "large_policy";
        let source = padded_rule_source(rule_set_id, 20_001, 60);
        assert!(source.len() > 1024 * 1024);
        assert!(source.len() < limits.max_policy_bytes.get());
        assert!(20_001 < limits.max_policy_lines.get());

        let edit_validation = validate_user_rule(&source, rule_set_id).unwrap();
        assert!(
            edit_validation.valid,
            "errors: {:?}",
            edit_validation.errors
        );

        let wrapper_validation = validate_runtime_wrapper(&source, rule_set_id).unwrap();
        assert!(
            wrapper_validation.valid,
            "errors: {:?}",
            wrapper_validation.errors
        );

        assert!(crate::UserRulesEngine::build(&[test_policy(rule_set_id, &source)]).is_ok());
    }

    #[test]
    fn release_policy_limits_reject_sources_over_line_limit_everywhere() {
        let limits = RuntimeLimits::release_defaults();
        let rule_set_id = "too_many_lines";
        let source = padded_rule_source(rule_set_id, limits.max_policy_lines.get(), 2);

        assert_validation_and_runtime_reject(rule_set_id, &source);
    }

    #[test]
    fn release_policy_limits_reject_sources_over_file_size_everywhere() {
        let limits = RuntimeLimits::release_defaults();
        let rule_set_id = "too_many_bytes";
        let comment_width = limits.max_policy_col.get() as usize;
        let comment_count = limits.max_policy_bytes.get() / (comment_width + 1) + 1;
        let source = padded_rule_source(rule_set_id, comment_count, comment_width);
        assert!(source.len() > limits.max_policy_bytes.get());

        assert_validation_and_runtime_reject(rule_set_id, &source);
    }

    #[test]
    fn release_policy_limits_preserve_default_column_limit_everywhere() {
        let limits = RuntimeLimits::release_defaults();
        let rule_set_id = "wide_line";
        let source = format!(
            "package scryer.rules.user.{rule_set_id}\n\
             import rego.v1\n\
             {}score_entry[\"bonus\"] := 100\n",
            " ".repeat(limits.max_policy_col.get() as usize),
        );

        assert_eq!(limits.max_policy_col.get(), 1024);
        assert_validation_and_runtime_reject(rule_set_id, &source);
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

    #[test]
    fn request_input_contract_copies_are_byte_identical() {
        assert_eq!(
            include_str!("../request-input-contract.json"),
            include_str!("../../../apps/scryer-web/lib/contracts/request-input-contract.json"),
            "crates/scryer-rules/request-input-contract.json and \
             apps/scryer-web/lib/contracts/request-input-contract.json must stay byte-identical"
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

    #[test]
    fn maintenance_source_and_loaded_module_fact_hooks_agree() {
        for (id, body) in [
            ("known_fact", "match if {\n  input.facts.monitored\n}\n"),
            (
                "dynamic_fact",
                "match if {\n  some fact\n  input.facts[fact]\n}\n",
            ),
        ] {
            let policy = maintenance_policy(id, body);
            let path = maintenance::user_policy_path(id);
            let module = parse_module(&policy.rego_source, &path).expect("source should parse");

            let source_result =
                <maintenance::MaintenanceFamily as crate::policy::PolicyFamily>::referenced_facts(
                    &policy, &path,
                );
            let module_result = <maintenance::MaintenanceFamily as crate::policy::PolicyFamily>::referenced_facts_from_module(
                &policy, &path, &module,
            );
            assert_eq!(source_result, module_result, "{id}");
        }
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
        let templates: [(&str, &str); 13] = [
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
                "expired-request-leases",
                "package rules\nimport rego.v1\n\nmatch if {\n\tinput.facts.request_lease_state == \"expired\"\n\tnot input.facts.keep_claim_active\n}\n",
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

    // ── request family ───────────────────────────────────────────────────────

    fn validate_request_body(id: &str, body: &str) -> ValidationResult {
        let source = request::rewrite_package_declaration(body, id);
        validate_request_rule(&source, id).expect("validation should not fail outright")
    }

    fn request_policy(id: &str, body: &str) -> request::RequestPolicy {
        request::RequestPolicy {
            id: id.to_string(),
            name: format!("rule {id}"),
            rego_source: request::rewrite_package_declaration(body, id),
        }
    }

    /// The engine is the second gate, exactly as it is for maintenance: a stored
    /// revision that somehow bypassed validation must still be refused at load,
    /// so the fact set the engine holds a rule on is never an underestimate.
    fn request_build_error(id: &str, body: &str) -> String {
        request::RequestRulesEngine::build(&[request_policy(id, body)])
            .err()
            .map(|error| error.to_string())
            .unwrap_or_else(|| panic!("engine build should have refused {id}"))
    }

    #[test]
    fn request_source_and_loaded_module_fact_hooks_agree() {
        for (id, body) in [
            (
                "known_fact",
                "approve if {\n  input.facts.certification_rank <= 2\n}\n",
            ),
            (
                "dynamic_fact",
                "approve if {\n  some fact\n  input.facts[fact]\n}\n",
            ),
        ] {
            let policy = request_policy(id, body);
            let path = request::user_policy_path(id);
            let module = parse_module(&policy.rego_source, &path).expect("source should parse");

            let source_result =
                <request::RequestFamily as crate::policy::PolicyFamily>::referenced_facts(
                    &policy, &path,
                );
            let module_result = <request::RequestFamily as crate::policy::PolicyFamily>::referenced_facts_from_module(
                &policy, &path, &module,
            );
            assert_eq!(source_result, module_result, "{id}");
        }
    }

    fn assert_request_rejected_everywhere(id: &str, body: &str, expected: &str) {
        let result = validate_request_body(id, body);
        assert!(!result.valid, "{id} should not validate");
        assert!(
            result.errors.iter().any(|error| error.contains(expected)),
            "{id} validation errors: {:?}",
            result.errors
        );

        let build_error = request_build_error(id, body);
        assert!(
            build_error.contains(expected),
            "{id} build error: {build_error}"
        );
    }

    #[test]
    fn valid_request_rule_passes_validation() {
        let result = validate_request_body(
            "family_rated",
            "approve if {\n  \
               input.facts.certification_rank <= 2\n  \
               input.request.lease_days <= 30\n\
             }\n\n\
             tags contains \"family\"\n\n\
             reasons contains \"family_rated\"\n",
        );
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    /// Every example the web offers as a starting point must validate exactly as
    /// written; a template that needs editing before it saves is not a template.
    #[test]
    fn every_pinned_request_example_validates() {
        for (index, (template_id, rego_source)) in
            request::REQUEST_RULE_EXAMPLES.into_iter().enumerate()
        {
            let rule_id = format!("request_template_{index}");
            let rewritten = request::rewrite_package_declaration(rego_source, &rule_id);
            let result = validate_request_rule(&rewritten, &rule_id).expect("validates");
            assert!(result.valid, "{template_id}: {:?}", result.errors);
        }
    }

    /// The wrapper defaults every head, so a rule that defines none of the four
    /// would compile, load, and abstain forever. Validation is the only place
    /// that can tell the author their rule can never say anything.
    #[test]
    fn request_rule_that_can_never_vote_is_rejected() {
        let result = validate_request_body(
            "reasons_only",
            "reasons contains \"adult_content\" if {\n  input.facts.is_adult\n}\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("can never vote")
                && result.errors[0].contains("'approve'")
                && result.errors[0].contains("'tags'"),
            "{:?}",
            result.errors
        );
    }

    /// A rule that only tags is a real rule: it contributes no vote but does
    /// change the title, so it must not be caught by the check above.
    #[test]
    fn request_rule_that_only_tags_is_accepted() {
        let result = validate_request_body(
            "tags_only",
            "tags contains \"family\" if {\n  input.facts.certification_rank <= 1\n}\n",
        );
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn request_rule_with_unknown_input_path_is_rejected() {
        let result = validate_request_body(
            "unknown_fact",
            "approve if {\n  input.facts.parental_score == 0\n}\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("Unknown rule input path 'input.facts.parental_score'")
                && result.errors[0].contains("Rules Context Reference"),
            "{:?}",
            result.errors
        );
    }

    /// The referenced-fact set is what makes an unobservable fact hold a rule,
    /// so a fact name the host cannot read off the source is refused outright.
    #[test]
    fn request_rule_with_dynamic_fact_access_is_rejected() {
        assert_request_rejected_everywhere(
            "dynamic_fact",
            "approve if {\n  some fact\n  input.facts[fact]\n}\n",
            "Unsupported dynamic fact access",
        );
    }

    #[test]
    fn request_rule_reading_the_whole_fact_object_is_rejected() {
        assert_request_rejected_everywhere(
            "whole_object",
            "deny if {\n  object.get(input.facts, \"is_adult\", false)\n}\n",
            "Unsupported reference to the whole fact object",
        );
    }

    /// Handing the whole document around would let a rule reach both the facts
    /// and the *requester* without ever writing either path — which defeats the
    /// auto-hold and the person-targeting gate at the same time.
    #[test]
    fn request_rule_referencing_the_whole_input_document_is_rejected() {
        const EXPECTED: &str = "Unsupported reference to the whole input document";

        assert_request_rejected_everywhere(
            "input_alias",
            "approve if {\n  \
               doc := input\n  \
               doc.requester.username == \"alice\"\n\
             }\n",
            EXPECTED,
        );

        assert_request_rejected_everywhere(
            "input_object_get",
            "approve if {\n  object.get(input, [\"facts\", \"is_adult\"], false)\n}\n",
            EXPECTED,
        );
    }

    #[test]
    fn request_rule_importing_part_of_input_is_rejected() {
        let source = request::rewrite_package_declaration(
            "import input.facts\n\napprove if {\n  not facts.is_adult\n}\n",
            "imported_facts",
        );
        let result = validate_request_rule(&source, "imported_facts").expect("validates");
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("Unsupported import 'input.facts'")
                && result.errors[0].contains("in a request rule"),
            "{:?}",
            result.errors
        );
    }

    #[test]
    fn request_rule_with_non_boolean_head_is_rejected() {
        for (id, body, expected) in [
            (
                "numeric_approve",
                "approve := 42\n",
                "'approve' must be a boolean",
            ),
            (
                "string_deny",
                "deny := \"yes\"\n",
                "'deny' must be a boolean",
            ),
            (
                "numeric_manual",
                "manual := 1\n",
                "'manual' must be a boolean",
            ),
        ] {
            let result = validate_request_body(id, body);
            assert!(!result.valid, "{id}");
            assert!(
                result.errors[0].contains(expected),
                "{id}: {:?}",
                result.errors
            );
        }
    }

    /// The bounded decoders run in the dry-run too, so an author learns their
    /// tag is unusable at save time rather than at submit time.
    #[test]
    fn request_rule_emitting_a_malformed_tag_is_rejected() {
        let result = validate_request_body(
            "bad_tag",
            "approve := true\n\ntags contains \"scryer:managed\"\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("output error") && result.errors[0].contains("reserved"),
            "{:?}",
            result.errors
        );
    }

    #[test]
    fn request_rule_with_wrong_package_is_rejected() {
        let source = request::rewrite_package_declaration("approve := true\n", "actual_id");
        let result = validate_request_rule(&source, "expected_id").unwrap();
        assert!(!result.valid);
        assert!(result.errors[0].contains("package declaration"));
    }

    /// The advanced surface stays documented and reachable, including the nested
    /// certification list under `value[]`.
    #[test]
    fn a_request_rule_may_read_the_observation_envelope_directly() {
        let result = validate_request_body(
            "envelope_reader",
            "manual if {\n  \
               input.observations.certification_rank.status == \"unknown\"\n\
             }\n\n\
             deny if {\n  \
               input.observations.certifications.value[0].country == \"US\"\n  \
               input.observations.certifications.value[0].value == \"NC-17\"\n\
             }\n\n\
             reasons contains input.observations.certification_rank.reason\n",
        );
        assert!(result.valid, "errors: {:?}", result.errors);
    }

    /// Named array and object access on the always-known sections works, and the
    /// open-keyed maps (`ratings_by_source`, `external_ids`) are reachable
    /// through `object.get` on the named fact — which still names the fact, so
    /// the auto-hold keeps counting it.
    #[test]
    fn a_request_rule_may_read_open_keyed_maps_through_the_named_fact() {
        let result = validate_request_body(
            "open_maps",
            "approve if {\n  \
               object.get(input.facts.ratings_by_source, \"imdb\", 0) >= 7\n  \
               input.request.external_ids.tmdb != \"\"\n  \
               input.facts.certifications[0].value == \"PG-13\"\n  \
               \"request\" in input.requester.library_permissions\n\
             }\n",
        );
        assert!(result.valid, "errors: {:?}", result.errors);

        let referenced = request_referenced_facts(
            &request::rewrite_package_declaration(
                "approve if {\n  object.get(input.facts.ratings_by_source, \"imdb\", 0) >= 7\n}\n",
                "open_maps",
            ),
            "open_maps",
        )
        .expect("fact set resolves");
        assert!(
            referenced.contains("ratings_by_source"),
            "reading the map through object.get still names the fact: {referenced:?}"
        );
    }

    #[test]
    fn request_referenced_facts_lists_every_named_fact() {
        let facts = request_referenced_facts(
            &request::rewrite_package_declaration(
                "approve if {\n  \
                   input.facts.certification_rank <= 2\n  \
                   not input.facts.is_adult\n  \
                   input.facts.genres[0] == \"Comedy\"\n\
                 }\n",
                "fact_reader",
            ),
            "fact_reader",
        )
        .expect("fact set resolves");

        assert_eq!(
            facts,
            BTreeSet::from([
                "certification_rank".to_string(),
                "genres".to_string(),
                "is_adult".to_string(),
            ])
        );
    }

    /// The gate the application authors on: which named people the rule asks
    /// about, in the order it asks.
    #[test]
    fn request_person_targeted_paths_lists_what_the_rule_reads() {
        assert_eq!(
            request_person_targeted_paths(
                &request::rewrite_package_declaration(
                    request::EXAMPLE_NAMED_REQUESTERS_FAMILY_RATED,
                    "example_1"
                ),
                "example_1"
            )
            .expect("resolves"),
            vec!["input.requester.username".to_string()]
        );

        assert_eq!(
            request_person_targeted_paths(
                &request::rewrite_package_declaration(
                    request::EXAMPLE_DENY_ADULT_CONTENT,
                    "example_4"
                ),
                "example_4"
            )
            .expect("resolves"),
            Vec::<String>::new(),
            "a content-only rule asks about nobody"
        );

        assert_eq!(
            request_person_targeted_paths(
                &request::rewrite_package_declaration(
                    "approve if {\n  \
                       input.requester.username == \"alice\"\n  \
                       \"manage_users\" in input.requester.app_permissions\n  \
                       input.requester.username != \"\"\n  \
                       input.requester.linked_providers[0] == \"plex\"\n\
                     }\n",
                    "many_paths"
                ),
                "many_paths"
            )
            .expect("resolves"),
            vec![
                "input.requester.username".to_string(),
                "input.requester.app_permissions".to_string(),
                "input.requester.linked_providers".to_string(),
            ],
            "deduplicated, in the order the source reads them"
        );
    }

    /// Reading the requester document whole is at least as person-targeted as
    /// reading one field of it.
    #[test]
    fn request_person_targeted_paths_counts_the_whole_requester_document() {
        assert_eq!(
            request_person_targeted_paths(
                &request::rewrite_package_declaration(
                    "approve if {\n  count(input.requester) > 0\n}\n",
                    "whole_requester"
                ),
                "whole_requester"
            )
            .expect("resolves"),
            vec![request::PERSON_TARGETED_REQUEST_ROOT.to_string()]
        );
    }

    /// A source whose references cannot be resolved has no person list either,
    /// so the caller has to reject it rather than read an empty answer as
    /// "asks about nobody".
    #[test]
    fn request_person_targeted_paths_refuses_an_unresolvable_source() {
        let bare_input = request_person_targeted_paths(
            &request::rewrite_package_declaration(
                "approve if {\n  doc := input\n  doc.requester.username == \"alice\"\n}\n",
                "bare_input",
            ),
            "bare_input",
        )
        .expect_err("a bare input document hides the requester read");
        assert!(
            bare_input.contains("Unsupported reference to the whole input document"),
            "{bare_input}"
        );

        let imported = request_person_targeted_paths(
            &request::rewrite_package_declaration(
                "import input.requester\n\napprove if {\n  requester.username == \"alice\"\n}\n",
                "imported",
            ),
            "imported",
        )
        .expect_err("an import hides the requester read");
        assert!(imported.contains("Unsupported import"), "{imported}");
    }

    #[test]
    fn the_person_targeted_root_is_the_requester_document() {
        assert_eq!(request::PERSON_TARGETED_REQUEST_ROOT, "input.requester");
    }

    /// The `input.release.extra.<key>` escape hatch is release-only.
    #[test]
    fn request_rule_cannot_use_release_extra_paths() {
        let result = validate_request_body(
            "extra_path",
            "approve if {\n  input.release.extra.anything == \"x\"\n}\n",
        );
        assert!(!result.valid);
        assert!(
            result.errors[0].contains("Unknown rule input path 'input.release.extra.anything'"),
            "{:?}",
            result.errors
        );
    }

    /// Each family judges paths against its own catalog: a maintenance fact is
    /// not a request fact, and neither is a request fact a maintenance one.
    #[test]
    fn the_three_family_catalogs_do_not_leak_into_each_other() {
        let request_reading_a_maintenance_fact = validate_request_body(
            "maintenance_fact",
            "approve if {\n  input.facts.monitored\n}\n",
        );
        assert!(!request_reading_a_maintenance_fact.valid);
        assert!(
            request_reading_a_maintenance_fact.errors[0]
                .contains("Unknown rule input path 'input.facts.monitored'"),
            "{:?}",
            request_reading_a_maintenance_fact.errors
        );

        let maintenance_reading_a_request_fact = validate_maintenance_body(
            "request_fact",
            "match if {\n  input.facts.certification_rank <= 2\n}\n",
        );
        assert!(!maintenance_reading_a_request_fact.valid);
        assert!(
            maintenance_reading_a_request_fact.errors[0]
                .contains("Unknown rule input path 'input.facts.certification_rank'"),
            "{:?}",
            maintenance_reading_a_request_fact.errors
        );
    }

    #[test]
    fn retired_release_input_fields_uses_ast_references_and_static_aliases() {
        let source = r#"
package scryer.rules.user.retired
import rego.v1
import input.release as release

score_entry["legacy"] := 1 if {
  doc := input.release
  doc.guide_facts
  release["guide_facts"]
  object.get(input.release, "guide_facts", [])
}
"#;
        assert_eq!(
            retired_release_input_fields(source).expect("source should parse"),
            vec!["input.release.guide_facts"]
        );
        assert_eq!(
            retired_release_input_fields(
                "package retired\nimport rego.v1\nimport input.release.guide_facts as old\nscore_entry[\"x\"] := 1"
            ).unwrap(),
            vec!["input.release.guide_facts"]
        );

        let decoys = r#"
package scryer.rules.user.decoys
import rego.v1

# input.release.guide_facts is retired.
score_entry["guide_facts"] := 1 if { "input.release.guide_facts" == "input.release.guide_facts" }
"#;
        assert!(
            retired_release_input_fields(decoys)
                .expect("decoy source should parse")
                .is_empty()
        );
    }

    #[test]
    fn baseline_dependencies_reject_direct_alias_and_dynamic_later_reads() {
        let additional = vec!["later".to_string()];
        for source in [
            r#"
package scryer.rules.user.baseline
import rego.v1
score_entry["x"] := input.builtin_score if { true }
"#,
            r#"
package scryer.rules.user.baseline
import rego.v1
import data.scryer.rules.user.later as later_rule
score_entry["x"] := later_rule.score_entry["x"] if { true }
"#,
            r#"
package scryer.rules.user.baseline
import rego.v1
score_entry["x"] := value if {
  later_rule := data.scryer.rules.user.later
  value := later_rule.score_entry["x"]
}
"#,
            r#"
package scryer.rules.user.baseline
import rego.v1
score_entry["x"] := 1 if {
  key := "builtin_score"
  input[key]
}
"#,
            r#"
package scryer.rules.user.baseline
import rego.v1
score_entry["x"] := 1 if {
  key := "later"
  data.scryer.rules.user[key]
}
"#,
        ] {
            assert!(
                validate_baseline_dependencies(source, &additional).is_err(),
                "source should reject: {source}"
            );
        }

        let allowed = r#"
package scryer.rules.user.baseline
import rego.v1
score_entry["x"] := 1 if { input.profile.name == "Cinema" }
"#;
        assert!(validate_baseline_dependencies(allowed, &additional).is_ok());
    }

    #[test]
    fn baseline_dependencies_preserve_bounded_reads_and_reject_indirect_subtotals() {
        let additional = vec!["later".to_string()];
        for expression in [
            "data.scryer.rules.user.earlier.value",
            "object.get(input, [\"profile\", \"name\"], null)",
            "object.get(input.profile, \"name\", null)",
        ] {
            let source = format!("package test\nimport rego.v1\nvalue := {expression}");
            assert!(
                validate_baseline_dependencies(&source, &additional).is_ok(),
                "{source}"
            );
        }
        for expression in [
            "input",
            "data",
            "data.scryer",
            "data[input.profile.name].rules.user.later.value",
            "object.get(input, [\"builtin_score\", \"total\"], 0)",
            "object.get(input.profile, input.builtin_score.total, 0)",
        ] {
            let source = format!("package test\nimport rego.v1\nvalue := {expression}");
            assert!(
                validate_baseline_dependencies(&source, &additional).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn retired_fields_resolve_scoped_aliases_and_nested_object_get() {
        for body in [
            "first if { doc := input.release; doc.guide_facts }\nsecond if { doc := input.profile; doc.name }",
            "first if { doc := object.get(input, \"release\", {}); doc.guide_facts }",
            "first := object.get(input, [\"release\", \"guide_facts\"], [])",
            "first := object.get(object.get(input, \"release\", {}), \"guide_facts\", [])",
        ] {
            let source = format!("package test\nimport rego.v1\n{body}");
            assert_eq!(
                retired_release_input_fields(&source).unwrap(),
                vec!["input.release.guide_facts"],
                "{source}"
            );
        }
    }
}
