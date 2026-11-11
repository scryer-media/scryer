//! Compiles normalized custom formats into one Rego module.
//!
//! Arr regexes are .NET regexes matched case-insensitively. Plain ones become
//! `regex.match` calls. Lookarounds have no Rego equivalent, so a pattern that
//! uses them is lowered into relations over `[start, end]` offsets of the
//! original string, with each plain fragment between assertions regenerated
//! and matched against the slice it covers.

use super::codes;
use super::inspection::{ImportedFormat, MAX_SPECIFICATIONS, Specification};
use super::js::utf16_len;
use super::onig::{self, AssertionKind, Node};
use super::rego::{
    Expr, Rule, Term, array, call, call_term, compare, number, predicate, print_module, ref_terms,
    reference, single, text, var,
};
use super::specifications::{Compiled, compile_leaf};
use super::{ArrDiagnostic, ArrDiagnosticSeverity, ArrFormatStatus, ArrSourceApplication};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

const MAX_REGEX_LENGTH: usize = 8_192;

static BACKREFERENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\(?:[1-9]|k<)").expect("valid backreference precheck"));
static RE2_INCOMPATIBLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(\?>|(?:\*|\+|\?|\{[0-9]+(?:,[0-9]*)?\})\+|\\K|\(\?[x-]")
        .expect("valid RE2 precheck")
});

pub(super) struct FormatOutcome {
    pub(super) status: ArrFormatStatus,
    pub(super) diagnostics: Vec<ArrDiagnostic>,
}

pub(super) struct Translation {
    pub(super) rego_source: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) applied_facets: Vec<String>,
    pub(super) formats: Vec<FormatOutcome>,
    pub(super) diagnostics: Vec<ArrDiagnostic>,
}

fn diagnostic(
    code: &'static str,
    message: impl Into<String>,
    format_id: &str,
    specification_index: Option<usize>,
) -> ArrDiagnostic {
    ArrDiagnostic {
        code,
        severity: ArrDiagnosticSeverity::Error,
        message: message.into(),
        location: None,
        format_id: Some(format_id.to_string()),
        specification_index,
    }
}

fn identifier(value: &str) -> String {
    let lower = value.to_lowercase();
    let mut normalized = String::with_capacity(lower.len());
    let mut in_run = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
            normalized.push(c);
            in_run = false;
        } else if !in_run {
            normalized.push('_');
            in_run = true;
        }
    }
    let trimmed = normalized.trim_matches('_');
    if trimmed.starts_with(|c: char| c.is_ascii_lowercase() || c == '_') {
        trimmed.to_string()
    } else {
        "imported_format".to_string()
    }
}

fn specification_name(format_index: usize, spec_index: usize) -> String {
    format!("_arr_{format_index}_spec_{spec_index}")
}

fn group_name(format_index: usize, group_index: usize) -> String {
    format!("_arr_{format_index}_group_{group_index}")
}

fn format_name(format_index: usize) -> String {
    format!("_arr_{format_index}_matches")
}

fn regex_failure(
    code: &'static str,
    message: impl Into<String>,
    format_id: &str,
    spec: &Specification,
) -> Compiled {
    Compiled::failed(diagnostic(code, message, format_id, Some(spec.index)))
}

/// The parser is the structural gate; constructs Rego's regex engine cannot
/// express never reach generated source.
pub(super) fn compile_regex(
    value: Option<&serde_json::Value>,
    input: &Term,
    format_id: &str,
    spec: &Specification,
    relation_prefix: &str,
) -> Compiled {
    let value = match value {
        Some(serde_json::Value::String(value)) if !value.is_empty() => value.as_str(),
        _ => {
            return regex_failure(
                codes::REGEX_VALUE_INVALID,
                "Regex specifications need a non-empty string value.",
                format_id,
                spec,
            );
        }
    };
    if value.chars().any(|c| (c as u32) < 32 || c as u32 == 127) {
        return regex_failure(
            codes::REGEX_CONTROL_CHARACTER,
            "Regex control characters cannot be safely emitted by the Rego deparser.",
            format_id,
            spec,
        );
    }
    if utf16_len(value) > MAX_REGEX_LENGTH {
        return regex_failure(
            codes::REGEX_TOO_LARGE,
            format!("Regex exceeds the {MAX_REGEX_LENGTH}-character import limit."),
            format_id,
            spec,
        );
    }
    // Backreferences require capture state, which position relations do not model.
    if BACKREFERENCE.is_match(value) {
        return regex_failure(
            codes::REGEX_BACKREFERENCE_UNSUPPORTED,
            "Backreferences cannot be represented by Rego's regex engine.",
            format_id,
            spec,
        );
    }
    if RE2_INCOMPATIBLE.is_match(value) {
        return regex_failure(
            codes::REGEX_RE2_INCOMPATIBLE,
            "This .NET regex construct has no equivalent in Rego's regex engine.",
            format_id,
            spec,
        );
    }
    // Arr accepts variable-width .NET lookbehind, which Oniguruma's own
    // validation would refuse; the lowering below handles any width.
    let parsed = onig::parse(value).and_then(|regex| {
        onig::check_capabilities(&regex)?;
        Ok(regex)
    });
    let regex = match parsed {
        Ok(regex) => regex,
        Err(detail) => {
            return regex_failure(
                codes::REGEX_PARSE_FAILED,
                format!("Regex could not be parsed: {detail}"),
                format_id,
                spec,
            );
        }
    };
    if regex.body.iter().any(contains_lookaround) {
        if regex.body.iter().any(has_quantified_assertion) {
            return regex_failure(
                codes::REGEX_QUANTIFIED_LOOKAROUND_UNSUPPORTED,
                "A quantified positional assertion cannot be lowered without unbounded expansion.",
                format_id,
                spec,
            );
        }
        let mut lowering = Lowering {
            input,
            prefix: relation_prefix,
            next: 0,
            rules: Vec::new(),
        };
        let name = lowering.lower(&regex.body);
        return Compiled {
            rules: lowering.rules,
            ..Compiled::ok(vec![single(ref_terms(vec![
                var(name),
                array(vec![var("_"), var("_")]),
            ]))])
        };
    }
    // Arr's custom-format regexes ignore case unless a specification carries
    // an explicit case-sensitive option, which its JSON export never does.
    Compiled::ok(vec![call(
        "regex.match",
        vec![text(format!("(?i){value}")), input.clone()],
    )])
}

fn contains_assertion(node: &Node) -> bool {
    matches!(node, Node::Assertion { .. } | Node::Lookaround { .. })
        || node.children().iter().any(contains_assertion)
}

fn contains_lookaround(node: &Node) -> bool {
    matches!(node, Node::Lookaround { .. }) || node.children().iter().any(contains_lookaround)
}

fn has_quantified_assertion(node: &Node) -> bool {
    match node {
        Node::Quantifier { body, .. } if contains_assertion(body) => true,
        _ => node.children().iter().any(has_quantified_assertion),
    }
}

/// Fixed widths avoid enumerating every substring for literal or group fragments.
fn fixed_width_all(nodes: &[Node]) -> Option<u64> {
    nodes
        .iter()
        .try_fold(0u64, |sum, node| Some(sum + fixed_width(node)?))
}

fn fixed_width(node: &Node) -> Option<u64> {
    match node {
        Node::Character(_) | Node::CharacterSet { .. } | Node::CharacterClass { .. } => Some(1),
        Node::Directive(_) => Some(0),
        Node::Alternative(body) => fixed_width_all(body),
        Node::Group { body, .. } | Node::CapturingGroup { body, .. } => {
            let widths: Vec<Option<u64>> = body.iter().map(fixed_width).collect();
            match widths.first() {
                Some(first) if widths.iter().all(|width| width == first) => *first,
                _ => None,
            }
        }
        Node::Quantifier { min, max, body, .. } if Some(*min) == *max => {
            fixed_width(body).map(|width| width * u64::from(*min))
        }
        _ => None,
    }
}

const POSITIONAL_ASSERTIONS: &[AssertionKind] = &[
    AssertionKind::WordBoundary,
    AssertionKind::LineStart,
    AssertionKind::StringStart,
    AssertionKind::SearchStart,
    AssertionKind::LineEnd,
    AssertionKind::StringEnd,
    AssertionKind::StringEndNewline,
];

struct Lowering<'a> {
    input: &'a Term,
    prefix: &'a str,
    next: usize,
    rules: Vec<Rule>,
}

fn relation_rule(name: &str, start: Term, end: Term, body: Vec<Expr>) -> Rule {
    Rule {
        name: name.to_string(),
        key: Some(array(vec![start, end])),
        value: Some(Term::Bool(true)),
        body,
    }
}

fn key(name: &str, start: Term, end: Term) -> Expr {
    single(ref_terms(vec![var(name), array(vec![start, end])]))
}

fn assign(name: &str, value: Term) -> Expr {
    compare("assign", var(name), value)
}

impl Lowering<'_> {
    fn make_name(&mut self) -> String {
        let name = format!("{}_position_{}", self.prefix, self.next);
        self.next += 1;
        name
    }

    fn count(&self) -> Term {
        call_term("count", vec![self.input.clone()])
    }

    fn input(&self) -> Term {
        self.input.clone()
    }

    /// `positions := numbers.range(0, count(input)); at := positions[_]`.
    fn enumerate(&self, name: &str) -> Vec<Expr> {
        let positions = format!("{name}_positions");
        vec![
            assign(
                &positions,
                call_term("numbers.range", vec![number(0.0), self.count()]),
            ),
            assign(
                &format!("{name}_at"),
                ref_terms(vec![var(&positions), var("_")]),
            ),
        ]
    }

    fn lower(&mut self, alternatives: &[Node]) -> String {
        let names: Vec<String> = alternatives
            .iter()
            .map(|alternative| self.lower_alternative(alternative))
            .collect();
        self.union(names)
    }

    fn union(&mut self, names: Vec<String>) -> String {
        if names.len() == 1 {
            return names.into_iter().next().expect("one alternative");
        }
        let name = self.make_name();
        let (start, end) = (format!("{name}_start"), format!("{name}_end"));
        for alternative in &names {
            self.rules.push(relation_rule(
                &name,
                var(&start),
                var(&end),
                vec![key(alternative, var(&start), var(&end))],
            ));
        }
        name
    }

    fn lower_alternative(&mut self, alternative: &Node) -> String {
        match alternative {
            Node::Alternative(body) => self.sequence(body),
            _ => self.sequence(&[]),
        }
    }

    fn fragment(&mut self, nodes: &[Node]) -> String {
        let name = self.make_name();
        let generated = onig::generate(nodes);
        let width = fixed_width_all(nodes);
        let start = format!("{name}_start");
        let end = format!("{name}_end");
        let starts = format!("{name}_starts");
        let ends = format!("{name}_ends");
        let slice = format!("{name}_slice");
        let length = format!("{name}_length");
        let mut body = vec![
            // A fragment absent from the whole input cannot match any substring.
            // Reject it before enumerating offsets and compiling per-slice regexes.
            call(
                "regex.match",
                vec![text(format!("(?i){generated}")), self.input()],
            ),
            assign(
                &starts,
                call_term("numbers.range", vec![number(0.0), self.count()]),
            ),
            assign(&start, ref_terms(vec![var(&starts), var("_")])),
        ];
        match width {
            None => {
                body.push(assign(
                    &ends,
                    call_term("numbers.range", vec![var(&start), self.count()]),
                ));
                body.push(assign(&end, ref_terms(vec![var(&ends), var("_")])));
            }
            Some(width) => {
                // Regorus has no named plus or minus functions; use aggregates.
                body.push(assign(
                    &end,
                    call_term("sum", vec![array(vec![var(&start), number(width as f64)])]),
                ));
                body.push(compare("lte", var(&end), self.count()));
            }
        }
        body.push(assign(
            &length,
            call_term(
                "sum",
                vec![array(vec![
                    var(&end),
                    call_term("product", vec![array(vec![number(-1.0), var(&start)])]),
                ])],
            ),
        ));
        body.push(assign(
            &slice,
            call_term("substring", vec![self.input(), var(&start), var(&length)]),
        ));
        body.push(call(
            "regex.match",
            vec![text(format!("(?i)\\A(?:{generated})\\z")), var(&slice)],
        ));
        self.rules
            .push(relation_rule(&name, var(&start), var(&end), body));
        name
    }

    fn assertion(&mut self, kind: AssertionKind, negate: bool) -> String {
        let name = self.make_name();
        let at = format!("{name}_at");
        let enumerate = self.enumerate(&name);
        let rule = |body: Vec<Expr>| relation_rule(&name, var(&at), var(&at), body);
        if kind == AssertionKind::WordBoundary {
            let prefix = format!("{name}_prefix");
            let suffix = format!("{name}_suffix");
            let mut context = enumerate;
            context.push(assign(
                &prefix,
                call_term("substring", vec![self.input(), number(0.0), var(&at)]),
            ));
            context.push(assign(
                &suffix,
                call_term("substring", vec![self.input(), var(&at), number(-1.0)]),
            ));
            let before = call("regex.match", vec![text("\\w\\z"), var(&prefix)]);
            let after = call("regex.match", vec![text("\\A\\w"), var(&suffix)]);
            // Use the target engine's word class at adjacent original offsets.
            // A boundary is XOR; a non-boundary is equality, without recursion.
            let mut first = context.clone();
            first.push(before.clone());
            first.push(after.clone().negated(!negate));
            let mut second = context;
            second.push(before.negated(true));
            second.push(after.negated(negate));
            self.rules.push(rule(first));
            self.rules.push(rule(second));
        } else {
            let at_start = matches!(
                kind,
                AssertionKind::LineStart | AssertionKind::StringStart | AssertionKind::SearchStart
            );
            let edge = if at_start { number(0.0) } else { self.count() };
            let mut body = enumerate.clone();
            body.push(compare("equal", var(&at), edge).negated(negate));
            self.rules.push(rule(body));
            // .NET `$` and `\Z` also match immediately before a final newline.
            if matches!(
                kind,
                AssertionKind::LineEnd | AssertionKind::StringEndNewline
            ) {
                let mut body = enumerate;
                body.push(compare(
                    "eq",
                    call_term("substring", vec![self.input(), var(&at), number(-1.0)]),
                    text("\n"),
                ));
                self.rules.push(rule(body));
            }
        }
        name
    }

    fn lookaround(&mut self, behind: bool, negate: bool, body: &[Node]) -> String {
        let alternatives: Vec<String> = body
            .iter()
            .map(|alternative| self.lower_alternative(alternative))
            .collect();
        let name = self.make_name();
        let at = format!("{name}_at");
        let enumerate = self.enumerate(&name);
        let anchored = |alternative: &str| {
            if behind {
                key(alternative, var("_"), var(&at))
            } else {
                key(alternative, var(&at), var("_"))
            }
        };
        if negate {
            // `(?!a|b)` succeeds only where neither alternative matches.
            let mut rule_body = enumerate;
            rule_body.extend(
                alternatives
                    .iter()
                    .map(|alternative| anchored(alternative).negated(true)),
            );
            self.rules
                .push(relation_rule(&name, var(&at), var(&at), rule_body));
        } else {
            for alternative in &alternatives {
                let mut rule_body = enumerate.clone();
                rule_body.push(anchored(alternative));
                self.rules
                    .push(relation_rule(&name, var(&at), var(&at), rule_body));
            }
        }
        name
    }

    fn sequence(&mut self, nodes: &[Node]) -> String {
        let mut pieces: Vec<String> = Vec::new();
        let mut regular: Vec<Node> = Vec::new();
        for node in nodes {
            match node {
                Node::Assertion { kind, negate } if POSITIONAL_ASSERTIONS.contains(kind) => {
                    self.flush(&mut regular, &mut pieces);
                    let piece = self.assertion(*kind, *negate);
                    pieces.push(piece);
                }
                Node::Lookaround {
                    behind,
                    negate,
                    body,
                } => {
                    self.flush(&mut regular, &mut pieces);
                    let piece = self.lookaround(*behind, *negate, body);
                    pieces.push(piece);
                }
                // Groups keep no observable capture state once backreferences
                // are refused, so their alternatives share one relation.
                _ if node.has_array_body() && contains_assertion(node) => {
                    self.flush(&mut regular, &mut pieces);
                    let alternatives: Vec<String> = node
                        .children()
                        .iter()
                        .map(|alternative| self.lower_alternative(alternative))
                        .collect();
                    let name = self.make_name();
                    let (start, end) = (format!("{name}_start"), format!("{name}_end"));
                    for alternative in &alternatives {
                        self.rules.push(relation_rule(
                            &name,
                            var(&start),
                            var(&end),
                            vec![key(alternative, var(&start), var(&end))],
                        ));
                    }
                    pieces.push(name);
                }
                _ => regular.push(node.clone()),
            }
        }
        self.flush(&mut regular, &mut pieces);
        match pieces.len() {
            0 => self.fragment(&[]),
            1 => pieces.pop().expect("one piece"),
            count => {
                let name = self.make_name();
                let point = |index: usize| var(format!("{name}_p{index}"));
                let body = pieces
                    .iter()
                    .enumerate()
                    .map(|(index, piece)| key(piece, point(index), point(index + 1)))
                    .collect();
                self.rules
                    .push(relation_rule(&name, point(0), point(count), body));
                name
            }
        }
    }

    fn flush(&mut self, regular: &mut Vec<Node>, pieces: &mut Vec<String>) {
        if !regular.is_empty() {
            let nodes = std::mem::take(regular);
            let piece = self.fragment(&nodes);
            pieces.push(piece);
        }
    }
}

fn compile_specification(
    spec: &Specification,
    format: &ImportedFormat,
    relation_prefix: &str,
) -> Compiled {
    let input = |field: &str| reference(&["input", "release", field]);
    let unsupported_fields = |label: &str| {
        Compiled::failed(diagnostic(
            codes::FIELDS_UNSUPPORTED,
            format!("{label} regex fields are not supported."),
            &format.id,
            Some(spec.index),
        ))
    };
    let only_value = spec.fields.iter().all(|(key, _)| key == "value");
    match spec.implementation.as_str() {
        "ReleaseTitleSpecification" => {
            if !only_value {
                return unsupported_fields("Release title");
            }
            let title = if format.source == ArrSourceApplication::Radarr {
                call_term(
                    "regex.replace",
                    vec![input("raw_title"), text("\\s*(?:[<>?*|])"), text("")],
                )
            } else {
                input("raw_title")
            };
            compile_regex(
                spec.field("value"),
                &title,
                &format.id,
                spec,
                relation_prefix,
            )
        }
        "ReleaseGroupSpecification" => {
            if !only_value {
                return unsupported_fields("Release group");
            }
            compile_regex(
                spec.field("value"),
                &input("release_group"),
                &format.id,
                spec,
                relation_prefix,
            )
        }
        "EditionSpecification" => {
            if !only_value {
                return unsupported_fields("Edition");
            }
            if format.source != ArrSourceApplication::Radarr {
                return Compiled::failed(diagnostic(
                    codes::EDITION_UNMAPPED,
                    "Edition specifications are Radarr-specific.",
                    &format.id,
                    Some(spec.index),
                ));
            }
            compile_regex(
                spec.field("value"),
                &input("edition"),
                &format.id,
                spec,
                relation_prefix,
            )
        }
        _ => compile_leaf(spec, format).unwrap_or_else(|| {
            Compiled::failed(diagnostic(
                codes::IMPLEMENTATION_UNSUPPORTED,
                format!("Unsupported Arr implementation {}.", spec.implementation),
                &format.id,
                Some(spec.index),
            ))
        }),
    }
}

fn helper_rule(name: String, body: Vec<Expr>) -> Rule {
    Rule {
        name,
        key: None,
        value: None,
        body,
    }
}

fn comments(lines: &[String]) -> String {
    lines
        .iter()
        .map(|line| {
            let mut out = String::from("# ");
            let mut in_break = false;
            for c in line.chars() {
                if c == '\r' || c == '\n' {
                    if !in_break {
                        out.push(' ');
                    }
                    in_break = true;
                } else {
                    out.push(c);
                    in_break = false;
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Compiles normalized formats into Rego source.
pub(super) fn translate_formats(
    formats: &[ImportedFormat],
    scores: &HashMap<String, i32>,
) -> Translation {
    let mut diagnostics = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    let mut annotations = vec![
        "Generated from Arr custom formats. Review disabled entries before saving.".to_string(),
    ];
    let mut outcomes = Vec::with_capacity(formats.len());
    let mut has_radarr = false;
    let mut has_sonarr = false;

    for (format_index, format) in formats.iter().enumerate() {
        has_radarr |= format.source == ArrSourceApplication::Radarr;
        has_sonarr |= format.source == ArrSourceApplication::Sonarr;
        let mut local = format.inspection_diagnostics.clone();
        if format.specifications.len() > MAX_SPECIFICATIONS {
            local.push(diagnostic(
                codes::FORMAT_TOO_LARGE,
                format!("Format exceeds the {MAX_SPECIFICATIONS}-specification import limit."),
                &format.id,
                None,
            ));
        }
        if format.specifications.is_empty() {
            local.push(diagnostic(
                codes::SPECIFICATIONS_EMPTY,
                "A format needs at least one valid condition.",
                &format.id,
                None,
            ));
        }
        let compiled: Vec<Compiled> = format
            .specifications
            .iter()
            .take(MAX_SPECIFICATIONS)
            .enumerate()
            .map(|(position, spec)| {
                compile_specification(
                    spec,
                    format,
                    &format!("_arr_{format_index}_spec_{position}"),
                )
            })
            .collect();
        for item in &compiled {
            local.extend(item.diagnostics.iter().cloned());
        }
        let score = scores.get(&format.id).copied();
        if score.is_none() {
            local.push(diagnostic(
                codes::SCORE_INVALID,
                "Choose a signed 32-bit integer score before translating this format.",
                &format.id,
                None,
            ));
        }
        let disabled = local
            .iter()
            .any(|item| item.severity == ArrDiagnosticSeverity::Error);
        let score_key = format!("{}_{}", identifier(&format.name), format_index + 1);
        if disabled {
            annotations.push(format!(
                "{}: disabled ({})",
                format.name,
                local
                    .iter()
                    .map(|item| item.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
            rules.push(Rule {
                name: "score_entry".to_string(),
                key: Some(text(score_key)),
                value: Some(number(f64::from(score.unwrap_or(0)))),
                body: vec![single(Term::Bool(false))],
            });
            diagnostics.extend(local.iter().cloned());
            outcomes.push(FormatOutcome {
                status: ArrFormatStatus::Disabled,
                diagnostics: local,
            });
            continue;
        }

        let mut by_implementation: Vec<(&str, Vec<usize>)> = Vec::new();
        for (position, (spec, item)) in format
            .specifications
            .iter()
            .zip(compiled.iter())
            .enumerate()
        {
            match by_implementation
                .iter_mut()
                .find(|(implementation, _)| *implementation == spec.implementation)
            {
                Some((_, positions)) => positions.push(position),
                None => by_implementation.push((&spec.implementation, vec![position])),
            }
            rules.extend(item.rules.iter().cloned());
            rules.push(helper_rule(
                specification_name(format_index, position),
                item.body.clone(),
            ));
            for alternative in &item.alternatives {
                rules.push(helper_rule(
                    specification_name(format_index, position),
                    alternative.clone(),
                ));
            }
        }
        let matches = |index: usize| {
            predicate(
                &specification_name(format_index, index),
                format.specifications[index].negate && !compiled[index].handles_negation,
            )
        };
        for (group_index, (_, positions)) in by_implementation.iter().enumerate() {
            let required: Vec<usize> = positions
                .iter()
                .copied()
                .filter(|index| format.specifications[*index].required)
                .collect();
            // Arr's DidMatch needs every required spec and at least one spec in
            // the group; any required match already satisfies the latter.
            if required.is_empty() {
                for option in positions {
                    rules.push(helper_rule(
                        group_name(format_index, group_index),
                        vec![matches(*option)],
                    ));
                }
            } else {
                rules.push(helper_rule(
                    group_name(format_index, group_index),
                    required.iter().map(|index| matches(*index)).collect(),
                ));
            }
        }
        rules.push(helper_rule(
            format_name(format_index),
            (0..by_implementation.len())
                .map(|index| predicate(&group_name(format_index, index), false))
                .collect(),
        ));
        rules.push(Rule {
            name: "score_entry".to_string(),
            key: Some(text(score_key)),
            value: Some(number(f64::from(score.unwrap_or(0)))),
            body: vec![predicate(&format_name(format_index), false)],
        });
        outcomes.push(FormatOutcome {
            status: ArrFormatStatus::Translated,
            diagnostics: Vec::new(),
        });
    }

    let rego = print_module(&rules);
    let applied_facets: &[&str] = match (has_radarr, has_sonarr) {
        (true, true) => &["movie", "series", "anime"],
        (true, false) => &["movie"],
        _ => &["series", "anime"],
    };
    Translation {
        rego_source: format!("{rego}\n\n{}\n", comments(&annotations)),
        name: if formats.len() == 1 {
            formats[0].name.clone()
        } else {
            "Imported Arr custom formats".to_string()
        },
        description: "Generated from pasted Arr custom-format JSON.".to_string(),
        applied_facets: applied_facets
            .iter()
            .map(|facet| facet.to_string())
            .collect(),
        formats: outcomes,
        diagnostics,
    }
}
