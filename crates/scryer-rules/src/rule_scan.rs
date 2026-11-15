//! Static scan machinery for families whose rules declare action heads in
//! their own source. The request family uses it; the lifecycle family keeps
//! its own copy in `maintenance_actions` until the two are folded together.
//!
//! A family scan reads a parsed module without running it: which heads the
//! source defines, which literal values it writes for them, and which shapes
//! it refuses. The pieces here are the family-independent half of that:
//! where an expression starts in terms an author can find, what a rule's head
//! names, which rule names the module refers to, and two refusals every
//! family applies, a head written as a function and a read of another
//! package's document.

use std::collections::HashSet;

use regorus::unstable::{Expr, Module, Rule, RuleHead};
use serde::Serialize;

use crate::diagnostic_codes;
use crate::validation::{static_reference, visit_rule_expr_nodes};

/// A literal value written for a value head, with where it was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleLiteral {
    pub value: String,
    /// One-based line and character column of the literal's opening quote.
    pub line: usize,
    pub column: usize,
}

/// A condition a scan found, located where the author would look for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleScanIssue {
    /// One of the codes in [`crate::diagnostic_codes`].
    pub code: &'static str,
    /// The rule name the condition concerns.
    pub rule: String,
    /// Display text. Code never reads it; the code and the fields carry the
    /// facts.
    pub message: String,
    /// One-based line and character column.
    pub line: usize,
    pub column: usize,
}

pub(crate) fn issue(
    code: &'static str,
    rule: &str,
    message: String,
    at: (usize, usize),
) -> RuleScanIssue {
    RuleScanIssue {
        code,
        rule: rule.to_string(),
        message,
        line: at.0,
        column: at.1,
    }
}

/// One-based line and character column where an expression starts.
///
/// Regorus places a reference or a call at its last component and a string
/// inside its quotes, so this walks back to the leftmost part and steps onto
/// the opening quote: the position an author would point at. The column is
/// counted in characters from the start of the line, because regorus counts
/// a tab as four columns and an editor counts it as one character.
pub(crate) fn source_position(expr: &Expr) -> (usize, usize) {
    match expr {
        Expr::RefDot { refr, .. } | Expr::RefBrack { refr, .. } => source_position(refr),
        Expr::Call { fcn, .. } => source_position(fcn),
        Expr::BinExpr { lhs, .. }
        | Expr::BoolExpr { lhs, .. }
        | Expr::ArithExpr { lhs, .. }
        | Expr::AssignExpr { lhs, .. }
        | Expr::OrExpr { lhs, .. } => source_position(lhs),
        Expr::String { span, .. } | Expr::RawString { span, .. } => {
            let contents = span.source.contents();
            let start = span.start as usize;
            let opening_quote = start
                .checked_sub(1)
                .and_then(|index| contents.as_bytes().get(index))
                .is_some_and(|byte| matches!(byte, b'"' | b'`'));
            let start = if opening_quote { start - 1 } else { start };
            (span.line as usize, char_column(contents, start, span.col))
        }
        _ => {
            let span = expr.span();
            (
                span.line as usize,
                char_column(span.source.contents(), span.start as usize, span.col),
            )
        }
    }
}

/// The one-based character column of byte offset `start`, or regorus's own
/// column when the offset does not land on a character boundary.
fn char_column(contents: &str, start: usize, fallback: u32) -> usize {
    let Some(before) = contents.get(..start) else {
        return fallback as usize;
    };
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    before[line_start..].chars().count() + 1
}

/// The head reference of a rule: the name, with any `.key` or `[key]`.
pub(crate) fn rule_refr(rule: &Rule) -> &Expr {
    match rule {
        Rule::Spec { head, .. } => match head {
            RuleHead::Compr { refr, .. }
            | RuleHead::Set { refr, .. }
            | RuleHead::Func { refr, .. } => refr,
        },
        Rule::Default { refr, .. } => refr,
    }
}

/// Whether the rule is a function definition rather than a document rule.
pub(crate) fn is_function_rule(rule: &Rule) -> bool {
    match rule {
        Rule::Spec { head, .. } => matches!(head, RuleHead::Func { .. }),
        Rule::Default { args, .. } => !args.is_empty(),
    }
}

/// The refusal for a function named after a head the host reads.
///
/// A function is not part of the package document, so it never reaches the
/// wrapper: `approve(x) if …` would look like a vote and never cast one.
pub(crate) fn head_defined_as_function(name: &str, at: (usize, usize)) -> RuleScanIssue {
    issue(
        diagnostic_codes::RULE_HEAD_DEFINED_AS_FUNCTION,
        name,
        format!(
            "'{name}' has a meaning to Scryer and cannot be a function; define it as a rule, for \
             example: {name} if {{ ... }}"
        ),
        at,
    )
}

pub(crate) fn string_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::String { value, .. } | Expr::RawString { value, .. } => {
            value.as_string().ok().map(|value| value.to_string())
        }
        _ => None,
    }
}

/// The value expressions one definition of a plain value head assigns, the
/// main branch first and then every `else` branch.
///
/// `Err` holds where to point when the definition is not one plain value: a
/// set rule, `name.key := …`, or a function builds the value out of pieces.
/// `None` in the list stands for a branch that assigns nothing and so yields
/// `true`.
pub(crate) fn plain_value_exprs<'a>(
    rule: &'a Rule,
    refr: &Expr,
) -> Result<Vec<Option<&'a Expr>>, (usize, usize)> {
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
        return Err(source_position(refr));
    }
    Ok(match rule {
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
    })
}

/// The module's package as document fields, without the leading `data`.
fn package_fields(module: &Module) -> Option<Vec<String>> {
    static_reference(&module.package.refr).map(|reference| reference.fields())
}

/// Every rule name the module refers to outside the heads that define it,
/// whether bare (`archive`), called (`helper(x)`), or through the package
/// document (`data.<package>.archive`).
pub(crate) fn referenced_names(module: &Module) -> HashSet<String> {
    let package = package_fields(module);
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
            let fields = reference.fields();
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

/// Every read of `data` that does not stay inside the rule's own package.
///
/// A rule's verdict must follow from its own source and its input. The data
/// document also holds the host's wrappers and every other rule, so a read of
/// it outside the rule's own package (a bare `data`, a computed path, another
/// package, or an import of one) is refused at the reference.
pub(crate) fn foreign_data_references(module: &Module) -> Vec<RuleScanIssue> {
    let package = package_fields(module).unwrap_or_default();
    let own = |fields: &[String]| {
        !package.is_empty()
            && fields.len() > package.len()
            && fields[1..=package.len()] == package[..]
    };
    let mut issues = Vec::new();
    for import in &module.imports {
        if let Some(reference) = static_reference(&import.refr) {
            let fields = reference.fields();
            if fields.first().map(String::as_str) == Some("data") && !own(&fields) {
                refuse(&import.refr, &mut issues);
            }
        }
    }
    for rule in &module.policy {
        visit_rule_expr_nodes(rule, |expr| {
            let Some(reference) = static_reference(expr) else {
                return;
            };
            let fields = reference.fields();
            if fields.first().map(String::as_str) == Some("data") && !own(&fields) {
                refuse(expr, &mut issues);
            }
        });
    }
    issues
}

fn refuse(expr: &Expr, issues: &mut Vec<RuleScanIssue>) {
    let at = source_position(expr);
    if issues.iter().any(|issue| (issue.line, issue.column) == at) {
        return;
    }
    issues.push(issue(
        diagnostic_codes::RULE_FOREIGN_DATA_REFERENCE,
        "data",
        "a rule may read data only inside its own package; read what it decides from input"
            .to_string(),
        at,
    ));
}
