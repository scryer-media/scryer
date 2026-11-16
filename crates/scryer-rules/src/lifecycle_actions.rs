//! Actions a lifecycle rule declares in its own source.
//!
//! A rule names what should happen to a subject by defining action heads in
//! its package: `unmonitor if { … }`, `add_tags := ["stale"] if { … }`, and
//! so on. Any other rule name is an author helper and means nothing to the
//! host. Two halves live here:
//!
//! - the runtime half, [`LifecycleActions`], is what one evaluation decided,
//!   decoded from the generated wrapper with the same fail-closed rules as the
//!   rest of the decision;
//! - the static half, [`LifecycleActionScan`], is what the source *can*
//!   decide, read off the parsed module without running it, so the host can
//!   check every profile name and tag a rule could ever apply before it runs.
//!
//! The host also loads a helper module into every rule's package that defines
//! `days_since(ts)`, the whole days between an RFC3339 timestamp and the
//! run's `input.evaluation_time`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use regorus::Value;
use regorus::unstable::{Expr, Literal, Module, Query, Rule, RuleHead};
use serde::{Deserialize, Serialize};

use crate::RulesError;
use crate::diagnostic_codes;
use crate::lifecycle::{self, USER_PACKAGE_PREFIX};
use crate::policy::decode::{MAX_TAGS, check_tag, decode_tag_list};
use crate::runtime::RuntimeLimits;
use crate::validation::{parse_module, rule_head_name, static_reference, visit_rule_expr_nodes};

// ── Action heads ────────────────────────────────────────────────────────────

/// One action head a lifecycle rule may define.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleActionHead {
    Unmonitor,
    DeleteFiles,
    DeleteTitle,
    Search,
    SetQualityProfile,
    AddTags,
    RemoveTags,
    Monitor,
    FetchSubtitles,
    MoveToLibrary,
    MoveToRoot,
    Notify,
}

/// The longest `notify` message, in bytes.
pub const MAX_NOTIFY_LEN: usize = 500;

impl LifecycleActionHead {
    /// Every action head, in the order the wrapper projects them.
    pub const ALL: [Self; 12] = [
        Self::Unmonitor,
        Self::DeleteFiles,
        Self::DeleteTitle,
        Self::Search,
        Self::SetQualityProfile,
        Self::AddTags,
        Self::RemoveTags,
        Self::Monitor,
        Self::FetchSubtitles,
        Self::MoveToLibrary,
        Self::MoveToRoot,
        Self::Notify,
    ];

    /// Every action head, in the order the host applies them. The builder
    /// offers them in this order too.
    pub const EXECUTION_ORDER: [Self; 12] = [
        Self::AddTags,
        Self::RemoveTags,
        Self::SetQualityProfile,
        Self::Monitor,
        Self::Search,
        Self::FetchSubtitles,
        Self::MoveToLibrary,
        Self::MoveToRoot,
        Self::Unmonitor,
        Self::DeleteFiles,
        Self::DeleteTitle,
        Self::Notify,
    ];

    /// The Rego rule name an author writes.
    pub const fn rule_name(self) -> &'static str {
        match self {
            Self::Unmonitor => "unmonitor",
            Self::DeleteFiles => "delete_files",
            Self::DeleteTitle => "delete_title",
            Self::Search => "search",
            Self::SetQualityProfile => "set_quality_profile",
            Self::AddTags => "add_tags",
            Self::RemoveTags => "remove_tags",
            Self::Monitor => "monitor",
            Self::FetchSubtitles => "fetch_subtitles",
            Self::MoveToLibrary => "move_to_library",
            Self::MoveToRoot => "move_to_root",
            Self::Notify => "notify",
        }
    }

    pub fn from_rule_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|head| head.rule_name() == name)
    }

    /// Whether the head carries a value rather than a plain yes.
    pub const fn carries_value(self) -> bool {
        self.carries_string() || matches!(self, Self::AddTags | Self::RemoveTags)
    }

    /// Whether the head's value is one string rather than a list.
    pub const fn carries_string(self) -> bool {
        matches!(
            self,
            Self::SetQualityProfile | Self::MoveToLibrary | Self::MoveToRoot | Self::Notify
        )
    }

    /// Whether the head removes media.
    pub const fn is_destructive(self) -> bool {
        matches!(self, Self::DeleteFiles | Self::DeleteTitle)
    }

    /// The heads that cannot be applied together with this one. Each pair is
    /// listed once, on the head that comes first in [`Self::EXECUTION_ORDER`].
    pub const fn conflicts_with(self) -> &'static [Self] {
        match self {
            Self::Monitor => &[Self::Unmonitor, Self::DeleteFiles, Self::DeleteTitle],
            Self::MoveToLibrary => &[Self::MoveToRoot, Self::DeleteFiles, Self::DeleteTitle],
            Self::MoveToRoot => &[Self::DeleteFiles, Self::DeleteTitle],
            _ => &[],
        }
    }

    /// The Rego literal the wrapper uses when the head is undefined.
    pub(crate) const fn wrapper_default(self) -> &'static str {
        match self {
            Self::Unmonitor
            | Self::DeleteFiles
            | Self::DeleteTitle
            | Self::Search
            | Self::Monitor
            | Self::FetchSubtitles => "false",
            Self::SetQualityProfile | Self::MoveToLibrary | Self::MoveToRoot | Self::Notify => {
                "null"
            }
            Self::AddTags | Self::RemoveTags => "[]",
        }
    }

    /// The example a refusal shows for a value head.
    fn value_example(self) -> &'static str {
        match self {
            Self::SetQualityProfile => "\"HD-1080p\"",
            Self::MoveToLibrary => "\"Movies\"",
            Self::MoveToRoot => "\"/media/movies\"",
            Self::Notify => "\"Archived to cold storage\"",
            _ => "[\"stale\"]",
        }
    }

    /// Why a string value written for this head is refused, if it is.
    fn check_string(self, value: &str) -> Option<String> {
        let name = self.rule_name();
        let blank = match self {
            Self::SetQualityProfile => value.is_empty(),
            _ => value.trim().is_empty(),
        };
        if blank {
            return Some(format!("'{name}' must not be empty"));
        }
        (self == Self::Notify && value.len() > MAX_NOTIFY_LEN).then(|| {
            format!(
                "'{name}' is {} bytes, at most {MAX_NOTIFY_LEN} are allowed",
                value.len()
            )
        })
    }
}

/// Rule names with a meaning to the host other than an action.
const CONTRACT_RULE_NAMES: [&str; 2] = ["unknown", "reasons"];

// ── Decoded actions ─────────────────────────────────────────────────────────

/// The actions one rule decided for one subject.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LifecycleActions {
    pub unmonitor: bool,
    pub delete_files: bool,
    pub delete_title: bool,
    pub search: bool,
    pub set_quality_profile: Option<String>,
    pub add_tags: Vec<String>,
    pub remove_tags: Vec<String>,
    // The heads below are left out of the JSON while unset, so a decision
    // that uses none of them keeps the form it was stored in before.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub monitor: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub fetch_subtitles: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_to_library: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_to_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify: Option<String>,
}

impl LifecycleActions {
    /// True when the rule decided nothing at all.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// True when the rule decided to delete files or the whole title.
    pub fn defines_destructive_action(&self) -> bool {
        self.delete_files || self.delete_title
    }

    /// The heads that fired, in wrapper order.
    pub fn heads(&self) -> Vec<LifecycleActionHead> {
        LifecycleActionHead::ALL
            .into_iter()
            .filter(|head| match head {
                LifecycleActionHead::Unmonitor => self.unmonitor,
                LifecycleActionHead::DeleteFiles => self.delete_files,
                LifecycleActionHead::DeleteTitle => self.delete_title,
                LifecycleActionHead::Search => self.search,
                LifecycleActionHead::SetQualityProfile => self.set_quality_profile.is_some(),
                LifecycleActionHead::AddTags => !self.add_tags.is_empty(),
                LifecycleActionHead::RemoveTags => !self.remove_tags.is_empty(),
                LifecycleActionHead::Monitor => self.monitor,
                LifecycleActionHead::FetchSubtitles => self.fetch_subtitles,
                LifecycleActionHead::MoveToLibrary => self.move_to_library.is_some(),
                LifecycleActionHead::MoveToRoot => self.move_to_root.is_some(),
                LifecycleActionHead::Notify => self.notify.is_some(),
            })
            .collect()
    }

    /// The first pair of decided heads that cannot be applied together, in
    /// execution order. A decision with one is held and applies nothing.
    pub fn conflict(&self) -> Option<(LifecycleActionHead, LifecycleActionHead)> {
        let heads = self.heads();
        LifecycleActionHead::EXECUTION_ORDER
            .into_iter()
            .filter(|head| heads.contains(head))
            .find_map(|head| {
                head.conflicts_with()
                    .iter()
                    .find(|other| heads.contains(other))
                    .map(|other| (head, *other))
            })
    }
}

/// Decode the action fields of the wrapper's decision object.
///
/// Fails closed: a defined head of the wrong type is an error for the rule,
/// never a coerced value.
pub(crate) fn decode_actions(value: &Value) -> Result<LifecycleActions, String> {
    let flag = |head: LifecycleActionHead| -> Result<bool, String> {
        value[head.rule_name()]
            .as_bool()
            .copied()
            .map_err(|_| format!("'{}' must be a boolean", head.rule_name()))
    };
    let string = |head: LifecycleActionHead| -> Result<Option<String>, String> {
        let name = head.rule_name();
        match &value[name] {
            Value::Null => Ok(None),
            text => {
                let text = text
                    .as_string()
                    .map_err(|_| format!("'{name}' must be a string"))?;
                if let Some(problem) = head.check_string(text) {
                    return Err(problem);
                }
                Ok(Some(text.to_string()))
            }
        }
    };
    let tags = |head: LifecycleActionHead| {
        let name = head.rule_name();
        // A list, as the contract says: a set would reorder what the author
        // wrote, and the static scan reads arrays only.
        if value[name].as_array().is_err() {
            return Err(format!("'{name}' must be an array of strings"));
        }
        decode_tag_list(&value[name], name)
    };

    Ok(LifecycleActions {
        unmonitor: flag(LifecycleActionHead::Unmonitor)?,
        delete_files: flag(LifecycleActionHead::DeleteFiles)?,
        delete_title: flag(LifecycleActionHead::DeleteTitle)?,
        search: flag(LifecycleActionHead::Search)?,
        set_quality_profile: string(LifecycleActionHead::SetQualityProfile)?,
        add_tags: tags(LifecycleActionHead::AddTags)?,
        remove_tags: tags(LifecycleActionHead::RemoveTags)?,
        monitor: flag(LifecycleActionHead::Monitor)?,
        fetch_subtitles: flag(LifecycleActionHead::FetchSubtitles)?,
        move_to_library: string(LifecycleActionHead::MoveToLibrary)?,
        move_to_root: string(LifecycleActionHead::MoveToRoot)?,
        notify: string(LifecycleActionHead::Notify)?,
    })
}

// ── Host helper module ──────────────────────────────────────────────────────

/// Name of the host-provided day-count helper.
pub const DAYS_SINCE_HELPER: &str = "days_since";

/// Nanoseconds in one day.
const DAY_NS: i64 = 24 * 60 * 60 * 1_000_000_000;

pub(crate) fn helper_policy_path(rule_id: &str) -> String {
    format!("internal/{rule_id}_maintenance_helpers.rego")
}

/// Source of the helper module loaded into a rule's own package.
///
/// It sits in the rule's package so an author calls `days_since(…)` without
/// qualifying it. The clock is `input.evaluation_time`, captured once per run
/// by the host: never `time.now_ns()`, which would let a retry decide
/// differently. An undefined argument (a fact Scryer could not observe is a
/// missing key) makes the call undefined, so the condition using it is false.
pub(crate) fn helper_module_source(rule_id: &str) -> String {
    format!(
        "package {USER_PACKAGE_PREFIX}.{rule_id}\n\
         import rego.v1\n\n\
         {DAYS_SINCE_HELPER}(ts) := floor((time.parse_rfc3339_ns(input.evaluation_time) - \
         time.parse_rfc3339_ns(ts)) / {DAY_NS}) if {{\n\
         \tis_string(ts)\n\
         }}\n"
    )
}

/// The helper modules to load beside a rule.
///
/// A rule that defines its own `days_since` keeps it and gets no host helper:
/// regorus merges function definitions across modules of one package, so two
/// definitions would conflict at evaluation. Validation refuses such a rule,
/// which leaves this branch for revisions stored before the helper existed.
pub(crate) fn helper_modules(rule_id: &str, module: &Module) -> Vec<(String, String)> {
    if module_defines_name(module, DAYS_SINCE_HELPER) {
        return Vec::new();
    }
    vec![(helper_policy_path(rule_id), helper_module_source(rule_id))]
}

fn module_defines_name(module: &Module, name: &str) -> bool {
    module
        .policy
        .iter()
        .any(|rule| rule_head_name(rule_refr(rule)) == Some(name))
}

fn rule_refr(rule: &Rule) -> &Expr {
    match rule {
        Rule::Spec { head, .. } => match head {
            RuleHead::Compr { refr, .. }
            | RuleHead::Set { refr, .. }
            | RuleHead::Func { refr, .. } => refr,
        },
        Rule::Default { refr, .. } => refr,
    }
}

// ── Static head scan ────────────────────────────────────────────────────────

/// A literal value written for a value head, with where it was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LifecycleActionLiteral {
    pub value: String,
    /// One-based line and column of the literal's opening quote.
    pub line: usize,
    pub column: usize,
}

/// A condition the scan found, located where the author would look for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LifecycleActionScanIssue {
    /// One of the codes in [`crate::diagnostic_codes`].
    pub code: &'static str,
    /// The rule name the condition concerns.
    pub rule: String,
    /// Display text. Code never reads it; the code and the fields carry the
    /// facts.
    pub message: String,
    pub line: usize,
    pub column: usize,
}

/// What a rule's source can decide, read without running it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LifecycleActionScan {
    /// Every action head the source defines, through any rule or `default`.
    pub defined_heads: BTreeSet<LifecycleActionHead>,
    /// Where each defined head is first written, as (line, column), so a
    /// refusal about the head itself can point at it.
    #[serde(skip)]
    pub head_positions: BTreeMap<LifecycleActionHead, (usize, usize)>,
    /// Every profile name written for `set_quality_profile`, across all
    /// definitions and `else` branches, in source order.
    pub quality_profiles: Vec<LifecycleActionLiteral>,
    /// Every tag written for `add_tags`, in source order.
    pub add_tags: Vec<LifecycleActionLiteral>,
    /// Every tag written for `remove_tags`, in source order.
    pub remove_tags: Vec<LifecycleActionLiteral>,
    /// Every library name written for `move_to_library`, in source order.
    pub libraries: Vec<LifecycleActionLiteral>,
    /// Every root path written for `move_to_root`, in source order.
    pub roots: Vec<LifecycleActionLiteral>,
    /// Every message written for `notify`, in source order.
    pub notifications: Vec<LifecycleActionLiteral>,
    /// Conditions that refuse the rule.
    pub errors: Vec<LifecycleActionScanIssue>,
    /// Conditions worth telling the author about that do not refuse the rule.
    pub warnings: Vec<LifecycleActionScanIssue>,
}

impl LifecycleActionScan {
    /// Whether the source defines `delete_files` or `delete_title`.
    pub fn defines_destructive_action(&self) -> bool {
        self.defined_heads.iter().any(|head| head.is_destructive())
    }

    fn literals_mut(&mut self, head: LifecycleActionHead) -> &mut Vec<LifecycleActionLiteral> {
        match head {
            LifecycleActionHead::SetQualityProfile => &mut self.quality_profiles,
            LifecycleActionHead::AddTags => &mut self.add_tags,
            LifecycleActionHead::MoveToLibrary => &mut self.libraries,
            LifecycleActionHead::MoveToRoot => &mut self.roots,
            LifecycleActionHead::Notify => &mut self.notifications,
            _ => &mut self.remove_tags,
        }
    }
}

/// Scan a stored lifecycle rule's source for the actions it can decide.
///
/// Fails only when the source does not parse; every other condition is an
/// issue on the result, and [`crate::validation::validate_lifecycle_rule`]
/// reports the same ones.
pub fn scan_lifecycle_action_heads(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<LifecycleActionScan, RulesError> {
    let module = parse_module(
        rego_source,
        &lifecycle::user_policy_path(rule_set_id),
        &RuntimeLimits::lifecycle_defaults(),
    )
    .map_err(RulesError::Compilation)?;
    Ok(scan_module(&module))
}

/// Why a loaded source must not be evaluated: its first scan error, or no
/// action head at all. The same conditions refuse it at save.
pub(crate) fn load_refusal(module: &Module) -> Option<crate::policy::PolicyRefusal> {
    let scan = scan_module(module);
    if let Some(error) = scan.errors.into_iter().next() {
        return Some(crate::policy::PolicyRefusal {
            code: error.code,
            message: error.message,
        });
    }
    scan.defined_heads
        .is_empty()
        .then(|| crate::policy::PolicyRefusal {
            code: diagnostic_codes::RULE_ACTION_REQUIRED,
            message: "lifecycle rule defines no action".to_string(),
        })
}

/// One-based line and column where an expression starts in its source.
///
/// Regorus places a reference or a call at its last component and a string
/// inside its quotes, so this walks back to the leftmost part and steps onto
/// the opening quote: the position an author would point at. The column
/// counts characters, as an editor does, not regorus's own columns.
fn source_position(expr: &Expr) -> (usize, usize) {
    match expr {
        Expr::RefDot { refr, .. } | Expr::RefBrack { refr, .. } => source_position(refr),
        Expr::Call { fcn, .. } => source_position(fcn),
        Expr::BinExpr { lhs, .. }
        | Expr::BoolExpr { lhs, .. }
        | Expr::ArithExpr { lhs, .. }
        | Expr::AssignExpr { lhs, .. }
        | Expr::OrExpr { lhs, .. } => source_position(lhs),
        Expr::String { span, .. } | Expr::RawString { span, .. } => {
            let start = span.start as usize;
            let opening_quote = start
                .checked_sub(1)
                .and_then(|index| span.source.contents().as_bytes().get(index))
                .is_some_and(|byte| matches!(byte, b'"' | b'`'));
            let start = if opening_quote { start - 1 } else { start };
            span_position(span, start)
        }
        _ => {
            let span = expr.span();
            span_position(span, span.start as usize)
        }
    }
}

fn span_position(span: &regorus::unstable::Span, byte_offset: usize) -> (usize, usize) {
    let contents = span.source.contents();
    match crate::validation::character_position(contents, byte_offset) {
        Some(position) => position,
        None => (span.line as usize, span.col as usize),
    }
}

fn issue(
    code: &'static str,
    rule: &str,
    message: String,
    at: (usize, usize),
) -> LifecycleActionScanIssue {
    LifecycleActionScanIssue {
        code,
        rule: rule.to_string(),
        message,
        line: at.0,
        column: at.1,
    }
}

/// Names an author may not define or import under, because the host helper
/// resolves them: the helper itself, and every name its body calls. Regorus
/// looks a call up in the package before the builtins, and an import alias
/// shadows a builtin namespace, so a rule named `floor` or an import aliased
/// `time` would quietly change what `days_since` returns.
const HOST_HELPER_NAMES: [&str; 4] = [DAYS_SINCE_HELPER, "floor", "is_string", "time"];

fn helper_redefined(name: &str, at: (usize, usize)) -> LifecycleActionScanIssue {
    issue(
        diagnostic_codes::RULE_HOST_HELPER_REDEFINED,
        name,
        format!(
            "'{name}' is used by Scryer's {DAYS_SINCE_HELPER} helper and cannot be redefined \
             or imported under; give your own name another spelling"
        ),
        at,
    )
}

/// Every `data` reference outside the rule's own package, once per place.
///
/// The unknown-fact hold reads the facts a rule references from its own
/// source. A read of another package's document, or of `data` as a whole,
/// would let a decision depend on something the hold never sees, so the
/// only `data` a rule may name is its own package's.
fn foreign_data_references(module: &Module) -> Vec<LifecycleActionScanIssue> {
    let package = static_reference(&module.package.refr)
        .map(|reference| reference.fields())
        .unwrap_or_default();
    let is_foreign = |expr: &Expr| {
        static_reference(expr).is_some_and(|reference| {
            let fields = reference.fields();
            fields.first().map(String::as_str) == Some("data")
                && !(fields.len() > package.len() && fields[1..=package.len()] == package[..])
        })
    };
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut issues = Vec::new();
    let mut report = |at: (usize, usize), issues: &mut Vec<LifecycleActionScanIssue>| {
        if seen.insert(at) {
            issues.push(issue(
                diagnostic_codes::RULE_FOREIGN_DATA_REFERENCE,
                "data",
                "a lifecycle rule may read only its own package under 'data'; use \
                 input.facts for what Scryer knows about the subject"
                    .to_string(),
                at,
            ));
        }
    };
    for import in &module.imports {
        if is_foreign(&import.refr) {
            report(source_position(&import.refr), &mut issues);
        }
    }
    for rule in &module.policy {
        visit_rule_expr_nodes(rule, |expr| {
            // Only the outermost reference: its parts share its position.
            if matches!(
                expr,
                Expr::Var { .. } | Expr::RefDot { .. } | Expr::RefBrack { .. }
            ) && is_foreign(expr)
            {
                report(source_position(expr), &mut issues);
            }
        });
    }
    issues
}

/// Every `with` that replaces the rule's input, its `data`, or a name the
/// host's helpers are built on, once per place.
///
/// The unknown-fact hold and the execution re-check both judge the facts the
/// host observed. A `with input...` or `with data...` would let a decision
/// rest on values the rule wrote itself, and a `with time...` (or another
/// helper name) would change what `days_since` computes.
fn protected_with_targets(module: &Module) -> Vec<LifecycleActionScanIssue> {
    fn check_query(query: &Query, issues: &mut Vec<LifecycleActionScanIssue>) {
        for statement in &query.stmts {
            for with_mod in &statement.with_mods {
                let target = rule_head_name(&with_mod.refr);
                if target.is_some_and(|name| {
                    name == "input" || name == "data" || HOST_HELPER_NAMES.contains(&name)
                }) {
                    let name = target.unwrap_or_default();
                    issues.push(issue(
                        diagnostic_codes::RULE_WITH_TARGET_NOT_ALLOWED,
                        name,
                        format!(
                            "'with' cannot replace '{name}' in a lifecycle rule; the rule \
                             must decide from what Scryer observed"
                        ),
                        source_position(&with_mod.refr),
                    ));
                }
            }
            if let Literal::Every { query, .. } = &statement.literal {
                check_query(query, issues);
            }
        }
    }

    let mut issues = Vec::new();
    for rule in &module.policy {
        if let Rule::Spec { bodies, .. } = rule.as_ref() {
            for body in bodies {
                check_query(&body.query, &mut issues);
            }
        }
        visit_rule_expr_nodes(rule, |expr| match expr {
            Expr::ArrayCompr { query, .. }
            | Expr::SetCompr { query, .. }
            | Expr::ObjectCompr { query, .. } => check_query(query, &mut issues),
            _ => {}
        });
    }
    let mut seen = BTreeSet::new();
    issues.retain(|issue| seen.insert((issue.line, issue.column)));
    issues
}

fn not_literal(head: LifecycleActionHead, at: (usize, usize)) -> LifecycleActionScanIssue {
    let name = head.rule_name();
    let example = head.value_example();
    let expected = if head.carries_string() {
        format!("a string literal, for example: {name} := {example} if {{ ... }}")
    } else {
        format!("an array of string literals, for example: {name} := {example} if {{ ... }}")
    };
    issue(
        diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
        name,
        format!("'{name}' must be assigned {expected}"),
        at,
    )
}

pub(crate) fn scan_module(module: &Module) -> LifecycleActionScan {
    let mut scan = LifecycleActionScan::default();
    // Every rule name the module defines, with where it is first defined.
    let mut defined: Vec<(String, (usize, usize))> = Vec::new();

    for rule in &module.policy {
        let refr = rule_refr(rule);
        let Some(name) = rule_head_name(refr) else {
            continue;
        };
        let at = source_position(refr);
        if !defined.iter().any(|(seen, _)| seen == name) {
            defined.push((name.to_string(), at));
        }
        if HOST_HELPER_NAMES.contains(&name) {
            scan.errors.push(helper_redefined(name, at));
            continue;
        }

        let Some(head) = LifecycleActionHead::from_rule_name(name) else {
            continue;
        };
        let is_function = match rule.as_ref() {
            Rule::Spec { head, .. } => matches!(head, RuleHead::Func { .. }),
            Rule::Default { args, .. } => !args.is_empty(),
        };
        if is_function {
            // A function of no arguments is evaluated into the package
            // document like any rule, so it would decide the action while the
            // scan could not read it. Every arity is refused alike.
            scan.errors.push(issue(
                diagnostic_codes::RULE_ACTION_HEAD_FUNCTION,
                name,
                format!(
                    "'{name}' is an action and cannot be written as a function; write it as a \
                     rule, for example: {name} if {{ ... }}"
                ),
                at,
            ));
            continue;
        }
        scan.defined_heads.insert(head);
        scan.head_positions.entry(head).or_insert(at);
        if head.carries_value() {
            scan_value_rule(&mut scan, head, rule, refr);
        }
    }

    for import in &module.imports {
        let (alias, at) = match &import.r#as {
            Some(alias) => (
                Some(alias.text().to_string()),
                span_position(alias, alias.start as usize),
            ),
            None => (
                static_reference(&import.refr).and_then(|reference| reference.fields().pop()),
                source_position(&import.refr),
            ),
        };
        if let Some(alias) = alias.filter(|alias| HOST_HELPER_NAMES.contains(&alias.as_str())) {
            scan.errors.push(helper_redefined(&alias, at));
        }
    }
    scan.errors.extend(foreign_data_references(module));
    scan.errors.extend(protected_with_targets(module));

    let referenced = referenced_names(module);
    for (name, at) in defined {
        if LifecycleActionHead::from_rule_name(&name).is_some()
            || CONTRACT_RULE_NAMES.contains(&name.as_str())
            || HOST_HELPER_NAMES.contains(&name.as_str())
            || referenced.contains(&name)
        {
            continue;
        }
        scan.warnings.push(issue(
            diagnostic_codes::RULE_UNREFERENCED_RULE,
            &name,
            format!(
                "'{name}' is not an action and nothing in this rule uses it; check the spelling \
                 if it was meant to be an action"
            ),
            at,
        ));
    }

    scan
}

/// Read one definition of a value head: record its literals, or refuse it.
fn scan_value_rule(
    scan: &mut LifecycleActionScan,
    head: LifecycleActionHead,
    rule: &Rule,
    refr: &Expr,
) {
    // The value has to be the whole rule under the plain name: a set rule or
    // `name.key := …` builds the value out of pieces.
    let plain = matches!(refr, Expr::Var { .. })
        && matches!(
            rule,
            Rule::Default { .. }
                | Rule::Spec {
                    head: RuleHead::Compr { .. },
                    ..
                }
        );
    if !plain {
        scan.errors.push(not_literal(head, source_position(refr)));
        return;
    }

    // `None` stands for a definition that assigns nothing, which yields `true`.
    let values: Vec<Option<&Expr>> = match rule {
        Rule::Spec {
            head: RuleHead::Compr { assign, .. },
            bodies,
            ..
        } => std::iter::once(assign.as_ref().map(|assign| assign.value.as_ref()))
            .chain(
                bodies
                    .iter()
                    .filter(|body| body.is_else)
                    .map(|body| body.assign.as_ref().map(|assign| assign.value.as_ref())),
            )
            .collect(),
        Rule::Default { value, .. } => vec![Some(value.as_ref())],
        Rule::Spec { .. } => Vec::new(),
    };

    for value in values {
        let Some(value) = value else {
            scan.errors.push(not_literal(head, source_position(refr)));
            continue;
        };
        let literals: Vec<&Expr> = if head.carries_string() {
            vec![value]
        } else {
            match value {
                Expr::Array { items, .. } => items.iter().map(|item| item.as_ref()).collect(),
                _ => {
                    scan.errors.push(not_literal(head, source_position(value)));
                    continue;
                }
            }
        };
        if literals.iter().any(|item| string_literal(item).is_none()) {
            scan.errors.push(not_literal(head, source_position(value)));
            continue;
        }
        // The same bound the decision decoder enforces, counted the same way:
        // entries as written, before duplicates fold.
        if !head.carries_string() && literals.len() > MAX_TAGS {
            scan.errors.push(issue(
                diagnostic_codes::RULE_ACTION_VALUE_INVALID,
                head.rule_name(),
                format!(
                    "'{}' has {} entries, at most {MAX_TAGS} are allowed",
                    head.rule_name(),
                    literals.len()
                ),
                source_position(value),
            ));
            continue;
        }

        for expr in literals {
            let Some(text) = string_literal(expr) else {
                continue;
            };
            let at = source_position(expr);
            let problem = if head.carries_string() {
                head.check_string(&text)
            } else {
                check_tag(&text).err()
            };
            match problem {
                Some(message) => scan.errors.push(issue(
                    diagnostic_codes::RULE_ACTION_VALUE_INVALID,
                    head.rule_name(),
                    message,
                    at,
                )),
                None => scan.literals_mut(head).push(LifecycleActionLiteral {
                    value: text,
                    line: at.0,
                    column: at.1,
                }),
            }
        }
    }
}

fn string_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::String { value, .. } | Expr::RawString { value, .. } => {
            value.as_string().ok().map(|value| value.to_string())
        }
        _ => None,
    }
}

/// Every rule name the module refers to outside the heads that define it,
/// whether bare (`archive`), called (`helper(x)`), or through the package
/// document (`data.<package>.archive`).
fn referenced_names(module: &Module) -> HashSet<String> {
    let package = static_reference(&module.package.refr).map(|reference| reference.fields());
    // A name an import binds stands for the path it imports, so a use of
    // `me.helper` after `import data.<own package> as me` uses `helper`.
    let imported: HashMap<String, Vec<String>> = module
        .imports
        .iter()
        .filter_map(|import| {
            let target = static_reference(&import.refr)?.fields();
            let alias = match &import.r#as {
                Some(alias) => alias.text().to_string(),
                None => target.last()?.clone(),
            };
            Some((alias, target))
        })
        .collect();
    let mut names = HashSet::new();
    for rule in &module.policy {
        let head_refr: *const Expr = rule_refr(rule);
        visit_rule_expr_nodes(rule, |expr| {
            if core::ptr::eq(expr, head_refr) {
                return;
            }
            let Some(reference) = static_reference(expr) else {
                return;
            };
            let mut fields = reference.fields();
            if let Some(target) = fields.first().and_then(|root| imported.get(root))
                && target.first().map(String::as_str) == Some("data")
            {
                // Keep the alias itself too: a rule may define a name that an
                // import also binds, and the plain reading must not be lost.
                names.insert(fields[0].clone());
                fields.splice(0..1, target.iter().cloned());
            }
            let name = match fields.first().map(String::as_str) {
                Some("data") => package.as_ref().and_then(|package| {
                    (fields.len() > package.len() + 1 && fields[1..=package.len()] == package[..])
                        .then(|| fields[package.len() + 1].clone())
                }),
                Some(root) => Some(root.to_string()),
                None => None,
            };
            if let Some(name) = name {
                names.insert(name);
            }
        });
    }
    names
}
