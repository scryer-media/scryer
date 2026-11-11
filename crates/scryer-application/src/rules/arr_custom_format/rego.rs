//! A small Rego AST and printer for translated Arr custom formats.
//!
//! Imported strings only ever reach string terms; variables and operators are
//! generated names. The printer keeps the layout the rule editor already
//! stores: `import rego.v1`, one blank line, then rules indented two spaces.

use super::js::js_number_to_string;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Term {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Var(String),
    Ref(Vec<Term>),
    Call(Vec<Term>),
    Array(Vec<Term>),
    Set(Vec<Term>),
    ArrayComprehension(Box<Term>, Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Terms {
    One(Term),
    Many(Vec<Term>),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Expr {
    pub(super) terms: Terms,
    pub(super) negated: bool,
}

impl Expr {
    pub(super) fn negated(mut self, negated: bool) -> Self {
        self.negated = negated;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Rule {
    pub(super) name: String,
    /// Parameters of a function rule; empty for every other rule.
    pub(super) args: Vec<Term>,
    pub(super) key: Option<Term>,
    pub(super) value: Option<Term>,
    pub(super) body: Vec<Expr>,
}

pub(super) fn text(value: impl Into<String>) -> Term {
    Term::Str(value.into())
}

pub(super) fn number(value: f64) -> Term {
    Term::Number(value)
}

pub(super) fn var(name: impl Into<String>) -> Term {
    Term::Var(name.into())
}

/// `root.a.b` with string path segments.
pub(super) fn reference(path: &[&str]) -> Term {
    Term::Ref(
        path.iter()
            .enumerate()
            .map(|(index, part)| if index == 0 { var(*part) } else { text(*part) })
            .collect(),
    )
}

pub(super) fn ref_terms(path: Vec<Term>) -> Term {
    Term::Ref(path)
}

pub(super) fn array(values: Vec<Term>) -> Term {
    Term::Array(values)
}

fn dotted(name: &str) -> Term {
    let parts: Vec<&str> = name.split('.').collect();
    reference(&parts)
}

pub(super) fn call_term(name: &str, args: Vec<Term>) -> Term {
    let mut terms = vec![dotted(name)];
    terms.extend(args);
    Term::Call(terms)
}

pub(super) fn call(name: &str, args: Vec<Term>) -> Expr {
    let mut terms = vec![dotted(name)];
    terms.extend(args);
    Expr {
        terms: Terms::Many(terms),
        negated: false,
    }
}

/// An infix comparison or assignment; `operator` is the deparser operator name.
pub(super) fn compare(operator: &str, left: Term, right: Term) -> Expr {
    Expr {
        terms: Terms::Many(vec![var(operator), left, right]),
        negated: false,
    }
}

pub(super) fn single(term: Term) -> Expr {
    Expr {
        terms: Terms::One(term),
        negated: false,
    }
}

pub(super) fn predicate(name: &str, negated: bool) -> Expr {
    single(var(name)).negated(negated)
}

/// `{choices...}[value]`, Rego's set membership lookup.
pub(super) fn member(value: Term, choices: Vec<Term>) -> Expr {
    single(Term::Ref(vec![Term::Set(choices), value]))
}

/// Prints a module with the `rego.v1` import.
pub(super) fn print_module(rules: &[Rule]) -> String {
    let mut parts = vec!["import rego.v1".to_string()];
    if !rules.is_empty() {
        parts.push(String::new());
        parts.extend(rules.iter().map(print_rule));
    }
    parts.join("\n")
}

fn print_rule(rule: &Rule) -> String {
    let mut out = rule.name.clone();
    if !rule.args.is_empty() {
        out.push('(');
        out.push_str(&join_terms(&rule.args));
        out.push(')');
    }
    if let Some(key) = &rule.key {
        out.push('[');
        out.push_str(&print_term(key));
        out.push(']');
    }
    if let Some(value) = &rule.value {
        out.push_str(" := ");
        out.push_str(&print_term(value));
    }
    if !rule.body.is_empty() {
        let body = rule
            .body
            .iter()
            .map(|expr| format!("  {}", print_expr(expr)))
            .collect::<Vec<_>>()
            .join("\n");
        if !body.is_empty() {
            out.push_str(" if {\n");
            out.push_str(&body);
            out.push_str("\n}");
        }
    }
    out
}

fn print_expr(expr: &Expr) -> String {
    let mut out = String::new();
    if expr.negated {
        out.push_str("not ");
    }
    match &expr.terms {
        Terms::One(term) => out.push_str(&print_term(term)),
        Terms::Many(terms) => out.push_str(&print_terms(terms)),
    }
    out
}

fn infix(name: &str) -> Option<&'static str> {
    Some(match name {
        "equal" => "==",
        "neq" => "!=",
        "lt" => "<",
        "lte" => "<=",
        "gt" => ">",
        "gte" => ">=",
        "plus" => "+",
        "minus" => "-",
        "mul" => "*",
        "div" => "/",
        "rem" => "%",
        "and" => "&",
        "or" => "|",
        "assign" => ":=",
        "eq" => "=",
        _ => return None,
    })
}

fn print_terms(terms: &[Term]) -> String {
    match terms {
        [] => String::new(),
        [only] => print_term(only),
        [first @ (Term::Ref(_) | Term::Var(_)), rest @ ..] => {
            let name = print_term_value(first);
            if let Some(symbol) = infix(&name) {
                if let [left, right] = rest {
                    return format!("{} {symbol} {}", print_term(left), print_term(right));
                }
                return rest
                    .iter()
                    .map(print_term)
                    .collect::<Vec<_>>()
                    .join(&format!(" {symbol} "));
            }
            format!("{name}({})", join_terms(rest))
        }
        _ => join_terms(terms),
    }
}

fn join_terms(terms: &[Term]) -> String {
    terms.iter().map(print_term).collect::<Vec<_>>().join(", ")
}

pub(super) fn print_term(term: &Term) -> String {
    match term {
        Term::Null => "null".to_string(),
        Term::Bool(value) => value.to_string(),
        Term::Number(value) => js_number_to_string(*value),
        Term::Str(value) => quote(value),
        Term::Var(name) => name.clone(),
        Term::Ref(path) => print_ref(path),
        Term::Call(terms) => match terms.split_first() {
            None => String::new(),
            Some((name, args)) => format!("{}({})", print_term_value(name), join_terms(args)),
        },
        Term::Array(values) => format!("[{}]", join_terms(values)),
        Term::Set(values) if values.is_empty() => "set()".to_string(),
        Term::Set(values) => format!("{{{}}}", join_terms(values)),
        Term::ArrayComprehension(term, body) => format!(
            "[{} | {}]",
            print_term(term),
            body.iter().map(print_expr).collect::<Vec<_>>().join("; ")
        ),
    }
}

fn print_term_value(term: &Term) -> String {
    match term {
        Term::Var(value) | Term::Str(value) => value.clone(),
        Term::Ref(path) => print_ref(path),
        Term::Number(value) => js_number_to_string(*value),
        Term::Bool(value) => value.to_string(),
        other => print_term(other),
    }
}

fn print_ref(path: &[Term]) -> String {
    let mut out = String::new();
    for (index, term) in path.iter().enumerate() {
        if index == 0 {
            out.push_str(&print_term_value(term));
            continue;
        }
        match term {
            Term::Str(key) if is_identifier(key) => {
                out.push('.');
                out.push_str(key);
            }
            Term::Str(key) => {
                out.push('[');
                out.push_str(&quote(key));
                out.push(']');
            }
            Term::Var(name) => {
                out.push('[');
                out.push_str(name);
                out.push(']');
            }
            other => {
                out.push('[');
                out.push_str(&print_term(other));
                out.push(']');
            }
        }
    }
    out
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Escapes backslash, quote, and the three whitespace controls, as the editor's
/// deparser always has. Other control characters are refused before printing.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
