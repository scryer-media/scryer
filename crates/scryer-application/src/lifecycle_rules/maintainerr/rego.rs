//! Turns one decoded rule group into lifecycle Rego.
//!
//! Every rule becomes a `maintainerr_condition_<n>` helper. Sections and the
//! group are folded left to right exactly as Maintainerr's comparator folds
//! them; where an OR is followed by an AND, the running result is named as a
//! step helper so the source stays linear in the number of rules.

use super::bind::{self, BindRefusal, Binding, Shape, Source};
use super::codes;
use super::decode::{
    Catalogue, CatalogueProperty, CustomValue, DecodedGroup, DecodedRule, MediaLevel, Operator,
};
use super::{
    ArrKind, LIFECYCLE_STARTER_SOURCE, MAINTAINERR_DESTRUCTIVE_MAX_TITLES_PER_RUN,
    MAX_MAINTAINERR_RULES_PER_GROUP, MaintainerrDiagnostic, MaintainerrDiagnosticSeverity,
    MaintainerrRuleStatus, MaintainerrTranslateContext, MaintainerrTranslatedRule,
};
use crate::lifecycle_rules::LIFECYCLE_MAX_GRACE_DAYS;
use scryer_rules::lifecycle::{PERSON_TARGETED_LIFECYCLE_FACTS, rewrite_package_declaration};
use scryer_rules::lifecycle_actions::LifecycleActionHead;
use scryer_rules::runtime::RuntimeLimits;
use scryer_rules::validation::{lifecycle_referenced_facts, validate_lifecycle_rule};
use std::collections::HashSet;

/// The rule id generated sources are validated under.
const VALIDATION_RULE_ID: &str = "maintainerr_import";
/// The longest comment text written into a source, in characters.
const MAX_COMMENT_CHARS: usize = 200;
/// The most failed parts a description names before summarising the rest.
const MAX_DESCRIBED_FAILURES: usize = 5;

// Maintainerr's RulePossibility values.
const BIGGER: i64 = 0;
const SMALLER: i64 = 1;
const EQUALS: i64 = 2;
const NOT_EQUALS: i64 = 3;
const CONTAINS: i64 = 4;
const BEFORE: i64 = 5;
const AFTER: i64 = 6;
const IN_LAST: i64 = 7;
const IN_NEXT: i64 = 8;
const NOT_CONTAINS: i64 = 9;
const CONTAINS_PARTIAL: i64 = 10;
const NOT_CONTAINS_PARTIAL: i64 = 11;
const CONTAINS_ALL: i64 = 12;
const NOT_CONTAINS_ALL: i64 = 13;
const COUNT_EQUALS: i64 = 14;
const COUNT_NOT_EQUALS: i64 = 15;
const COUNT_BIGGER: i64 = 16;
const COUNT_SMALLER: i64 = 17;
const EXISTS: i64 = 18;
const NOT_EXISTS: i64 = 19;

// Maintainerr's ServarrAction values.
const ACTION_DELETE: i64 = 0;
const ACTION_UNMONITOR_DELETE_ALL: i64 = 1;
const ACTION_UNMONITOR_DELETE_EXISTING: i64 = 2;
const ACTION_UNMONITOR: i64 = 3;
const ACTION_DO_NOTHING: i64 = 4;
const ACTION_DELETE_SHOW_IF_EMPTY: i64 = 5;
const ACTION_UNMONITOR_SHOW_IF_EMPTY: i64 = 6;
const ACTION_CHANGE_QUALITY_PROFILE: i64 = 7;

// Maintainerr's RuleType ids for custom values.
const CUSTOM_NUMBER: i64 = 0;
const CUSTOM_DATE: i64 = 1;
const CUSTOM_TEXT: i64 = 2;
const CUSTOM_BOOL: i64 = 3;
const CUSTOM_TEXT_LIST: i64 = 4;

/// The widest custom date or offset accepted, in seconds either side of the
/// epoch or of now: two hundred years keeps every sum inside an i64 of
/// nanoseconds.
const MAX_ABS_SECONDS: f64 = 200.0 * 365.25 * 86_400.0;
const NANOS_PER_MILLI: i128 = 1_000_000;

fn comparator_label(action: i64) -> &'static str {
    match action {
        BIGGER => "bigger",
        SMALLER => "smaller",
        EQUALS => "equals",
        NOT_EQUALS => "not equals",
        CONTAINS => "contains",
        BEFORE => "before",
        AFTER => "after",
        IN_LAST => "in last",
        IN_NEXT => "in next",
        NOT_CONTAINS => "not contains",
        CONTAINS_PARTIAL => "contains partial",
        NOT_CONTAINS_PARTIAL => "not contains partial",
        CONTAINS_ALL => "contains all",
        NOT_CONTAINS_ALL => "not contains all",
        COUNT_EQUALS => "count equals",
        COUNT_NOT_EQUALS => "count not equals",
        COUNT_BIGGER => "count bigger",
        COUNT_SMALLER => "count smaller",
        EXISTS => "exists",
        NOT_EXISTS => "not exists",
        _ => "an unknown comparison",
    }
}

fn action_label(action: i64) -> &'static str {
    match action {
        ACTION_DELETE => "Delete",
        ACTION_UNMONITOR_DELETE_ALL => "Unmonitor and delete all",
        ACTION_UNMONITOR_DELETE_EXISTING => "Unmonitor and delete existing",
        ACTION_UNMONITOR => "Unmonitor",
        ACTION_DO_NOTHING => "Do nothing",
        ACTION_DELETE_SHOW_IF_EMPTY => "Delete show if empty",
        ACTION_UNMONITOR_SHOW_IF_EMPTY => "Unmonitor show if empty",
        ACTION_CHANGE_QUALITY_PROFILE => "Change quality profile",
        _ => "an unknown action",
    }
}

/// Something that keeps a group from translating.
struct Failure {
    code: &'static str,
    /// A short phrase for the rule's description.
    subject: String,
    message: String,
    rule_index: Option<usize>,
}

impl Failure {
    fn group(code: &'static str, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            subject: subject.into(),
            message: message.into(),
            rule_index: None,
        }
    }

    fn rule(
        code: &'static str,
        index: usize,
        subject: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            subject: subject.into(),
            message: format!("Rule {}: {}", index + 1, message.into()),
            rule_index: Some(index),
        }
    }
}

fn warning(
    code: &'static str,
    message: String,
    rule_index: Option<usize>,
) -> MaintainerrDiagnostic {
    MaintainerrDiagnostic {
        code,
        severity: MaintainerrDiagnosticSeverity::Warning,
        message,
        rule_index,
    }
}

// ── Group ──────────────────────────────────────────────────────────────────

pub(super) fn translate_group(
    group: &DecodedGroup,
    catalogue: &Catalogue,
    ctx: &MaintainerrTranslateContext,
    known_usernames: &HashSet<String>,
) -> MaintainerrTranslatedRule {
    let display = display_name(group);
    let grace_days = grace_days(group.collection.delete_after_days);
    let mut warnings = Vec::new();
    let mut failures = Vec::new();

    if !group.use_rules {
        failures.push(Failure::group(
            codes::GROUP_RULES_DISABLED,
            "the group does not use rules",
            "The group does not use rules, so it has no conditions to translate.",
        ));
    }
    let rules = match &group.rules {
        Ok(rules) => rules.as_slice(),
        Err((index, why)) => {
            failures.push(Failure::rule(
                codes::RULE_INVALID,
                *index,
                format!("rule {} could not be read", index + 1),
                why.clone(),
            ));
            &[]
        }
    };
    if group.rules.is_ok() && rules.is_empty() {
        failures.push(Failure::group(
            codes::GROUP_RULES_EMPTY,
            "the group has no rules",
            "The group has no rules.",
        ));
    }
    let rules_fit = rules.len() <= MAX_MAINTAINERR_RULES_PER_GROUP;
    if !rules_fit {
        failures.push(Failure::group(
            codes::GROUP_RULES_TOO_MANY,
            format!("more than {MAX_MAINTAINERR_RULES_PER_GROUP} rules"),
            format!(
                "The group has {} rules; at most {MAX_MAINTAINERR_RULES_PER_GROUP} translate.",
                rules.len()
            ),
        ));
    }

    let mut conditions = Vec::new();
    let mut plan = None;
    match group.level {
        None => failures.push(Failure::group(
            codes::GROUP_MEDIA_TYPE_UNSUPPORTED,
            "the group's media type",
            "The group's media type is missing or is not movies, shows, seasons or episodes.",
        )),
        Some(level) => {
            match plan_action(group, level, ctx) {
                Ok((found, note)) => {
                    if let Some(note) = note {
                        warnings.push(warning(codes::ACTION_APPROXIMATE, note, None));
                    }
                    plan = Some(found);
                }
                Err(failure) => failures.push(failure),
            }
            if rules_fit {
                for (index, rule) in rules.iter().enumerate() {
                    match compile_rule(index, rule, level, catalogue, known_usernames) {
                        Ok(compiled) => {
                            warnings.extend(compiled.warnings);
                            conditions.push(compiled.rego);
                        }
                        Err(failure) => failures.push(failure),
                    }
                }
            }
        }
    }

    let (Some(level), Some(plan), true) = (group.level, plan, failures.is_empty()) else {
        return failed(group, &display, grace_days, failures, warnings);
    };
    let source = match render(group, &display, level, rules, &conditions, &plan) {
        Ok(source) => source,
        Err(failure) => return failed(group, &display, grace_days, vec![failure], warnings),
    };
    if let Some(failure) = exceeds_limits(&source) {
        return failed(group, &display, grace_days, vec![failure], warnings);
    }
    let checked = match check_source(&source) {
        Ok(checked) => checked,
        Err(failure) => return failed(group, &display, grace_days, vec![failure], warnings),
    };

    let destructive = plan.heads.iter().any(|(head, _)| head.is_destructive());
    MaintainerrTranslatedRule {
        source_group_id: group.id,
        source_group_name: group.name.clone(),
        source_library_id: group.library_id.clone(),
        source_is_active: group.is_active,
        name: format!("(Maintainerr) {display}"),
        description: with_group_description(
            format!("Imported from the Maintainerr rule group \"{display}\"."),
            group,
        ),
        rego_source: source,
        grace_days,
        max_titles_per_run: destructive.then_some(MAINTAINERR_DESTRUCTIVE_MAX_TITLES_PER_RUN),
        status: MaintainerrRuleStatus::Translated,
        reads_person_targeted_fact: checked,
        diagnostics: warnings,
    }
}

fn failed(
    group: &DecodedGroup,
    display: &str,
    grace_days: i64,
    failures: Vec<Failure>,
    warnings: Vec<MaintainerrDiagnostic>,
) -> MaintainerrTranslatedRule {
    let mut subjects: Vec<&str> = Vec::new();
    for failure in &failures {
        if !subjects.contains(&failure.subject.as_str()) {
            subjects.push(&failure.subject);
        }
    }
    let mut summary = subjects
        .iter()
        .take(MAX_DESCRIBED_FAILURES)
        .copied()
        .collect::<Vec<_>>()
        .join("; ");
    if subjects.len() > MAX_DESCRIBED_FAILURES {
        summary.push_str(&format!(
            "; and {} more",
            subjects.len() - MAX_DESCRIBED_FAILURES
        ));
    }
    let mut diagnostics: Vec<MaintainerrDiagnostic> = failures
        .into_iter()
        .map(|failure| MaintainerrDiagnostic {
            code: failure.code,
            severity: MaintainerrDiagnosticSeverity::Error,
            message: failure.message,
            rule_index: failure.rule_index,
        })
        .collect();
    diagnostics.extend(warnings);
    MaintainerrTranslatedRule {
        source_group_id: group.id,
        source_group_name: group.name.clone(),
        source_library_id: group.library_id.clone(),
        source_is_active: group.is_active,
        name: format!("(Maintainerr) FAILED - {display}"),
        description: with_group_description(
            format!("Could not translate from Maintainerr: {summary}."),
            group,
        ),
        rego_source: LIFECYCLE_STARTER_SOURCE.to_string(),
        grace_days,
        max_titles_per_run: None,
        status: MaintainerrRuleStatus::Failed,
        reads_person_targeted_fact: false,
        diagnostics,
    }
}

fn with_group_description(sentence: String, group: &DecodedGroup) -> String {
    match group
        .description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        Some(own) => format!("{sentence}\n\n{own}"),
        None => sentence,
    }
}

fn display_name(group: &DecodedGroup) -> String {
    let cleaned = single_line(&group.name);
    if cleaned.is_empty() {
        format!("rule group {}", group.id)
    } else {
        cleaned
    }
}

fn grace_days(delete_after_days: Option<f64>) -> i64 {
    let days = delete_after_days.unwrap_or(0.0).trunc();
    days.clamp(0.0, LIFECYCLE_MAX_GRACE_DAYS as f64) as i64
}

/// Text with every control or line-breaking character replaced by a space.
fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn comment(text: &str) -> String {
    let line = single_line(text);
    let mut out: String = line.chars().take(MAX_COMMENT_CHARS).collect();
    if line.chars().count() > MAX_COMMENT_CHARS {
        out.push_str("...");
    }
    out
}

/// A Rego string literal. JSON string escapes are Rego string escapes.
fn literal(text: &str) -> String {
    serde_json::to_string(text).expect("a string always serializes")
}

fn number_literal(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1.0e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn list_literal(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| literal(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ── Action ─────────────────────────────────────────────────────────────────

struct ActionPlan {
    heads: Vec<(LifecycleActionHead, Option<String>)>,
    label: &'static str,
}

fn plan_action(
    group: &DecodedGroup,
    level: MediaLevel,
    ctx: &MaintainerrTranslateContext,
) -> Result<(ActionPlan, Option<String>), Failure> {
    use LifecycleActionHead as H;
    let Some(action) = group.collection.arr_action else {
        return Err(Failure::group(
            codes::ACTION_MISSING,
            "the group has no action",
            "The group's collection has no action.",
        ));
    };
    let label = action_label(action);
    let unsupported = || {
        Failure::group(
            codes::ACTION_UNSUPPORTED,
            format!("the action \"{label}\" for {}", level.label()),
            format!(
                "Scryer has no equivalent of the action \"{label}\" for {}.",
                level.label()
            ),
        )
    };
    let notify = || {
        let mut text = format!(
            "Matched the Maintainerr rule group \"{}\".",
            display_name(group)
        );
        truncate_bytes(&mut text, scryer_rules::lifecycle_actions::MAX_NOTIFY_LEN);
        Some(text)
    };
    let mut note = None;
    let heads = match (level, action) {
        (MediaLevel::Movie | MediaLevel::Show, ACTION_DELETE) => vec![(H::DeleteTitle, None)],
        (MediaLevel::Movie, ACTION_UNMONITOR_DELETE_EXISTING) => vec![(H::DeleteTitle, None)],
        (MediaLevel::Movie | MediaLevel::Show, ACTION_UNMONITOR_DELETE_ALL) => {
            vec![(H::Unmonitor, None), (H::DeleteFiles, None)]
        }
        (MediaLevel::Show, ACTION_UNMONITOR_DELETE_EXISTING) => {
            note = Some(
                "Maintainerr unmonitors only the seasons that had files; Scryer unmonitors the whole show."
                    .to_string(),
            );
            vec![(H::Unmonitor, None), (H::DeleteFiles, None)]
        }
        (MediaLevel::Movie | MediaLevel::Show, ACTION_UNMONITOR) => vec![(H::Unmonitor, None)],
        (MediaLevel::Season, ACTION_DELETE | ACTION_UNMONITOR_DELETE_EXISTING) => {
            vec![(H::UnmonitorSeason, None), (H::DeleteSeasonFiles, None)]
        }
        (MediaLevel::Season, ACTION_UNMONITOR) => vec![(H::UnmonitorSeason, None)],
        (MediaLevel::Episode, ACTION_DELETE) => {
            vec![(H::UnmonitorEpisode, None), (H::DeleteEpisodeFiles, None)]
        }
        (MediaLevel::Episode, ACTION_UNMONITOR) => vec![(H::UnmonitorEpisode, None)],
        (_, ACTION_DO_NOTHING) => {
            note = Some(
                "Maintainerr leaves matches alone; Scryer sends a notification for each match instead."
                    .to_string(),
            );
            let head = match level {
                MediaLevel::Movie | MediaLevel::Show => H::Notify,
                MediaLevel::Season => H::NotifySeason,
                MediaLevel::Episode => H::NotifyEpisode,
            };
            vec![(head, notify())]
        }
        (MediaLevel::Movie | MediaLevel::Show, ACTION_CHANGE_QUALITY_PROFILE) => {
            let profile = resolve_profile(group, level, ctx)?;
            vec![(H::SetQualityProfile, Some(profile)), (H::Search, None)]
        }
        _ => return Err(unsupported()),
    };
    Ok((ActionPlan { heads, label }, note))
}

fn resolve_profile(
    group: &DecodedGroup,
    level: MediaLevel,
    ctx: &MaintainerrTranslateContext,
) -> Result<String, Failure> {
    let collection = &group.collection;
    let (kind, settings, profile) = match level {
        MediaLevel::Movie => (
            ArrKind::Radarr,
            collection.radarr_settings_id,
            collection.radarr_quality_profile_id,
        ),
        _ => (
            ArrKind::Sonarr,
            collection.sonarr_settings_id,
            collection.sonarr_quality_profile_id,
        ),
    };
    let unresolved = || {
        Failure::group(
            codes::QUALITY_PROFILE_UNRESOLVED,
            "the quality profile to change to",
            format!("The quality profile the group changes to is not one {kind:?} lists."),
        )
    };
    let (Some(settings), Some(profile)) = (settings, profile) else {
        return Err(unresolved());
    };
    let name = ctx
        .arr_profile_names
        .get(&(kind, settings, profile))
        .ok_or_else(unresolved)?;
    if !ctx.scryer_quality_profile_names.contains(name) {
        return Err(Failure::group(
            codes::QUALITY_PROFILE_NOT_IN_SCRYER,
            format!("the quality profile \"{}\"", single_line(name)),
            format!(
                "No Scryer quality profile is named \"{}\".",
                single_line(name)
            ),
        ));
    }
    Ok(name.clone())
}

fn truncate_bytes(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

// ── Rules ──────────────────────────────────────────────────────────────────

struct CompiledRule {
    rego: String,
    warnings: Vec<MaintainerrDiagnostic>,
}

/// A value read from the input: `guard` proves it is present and of its
/// shape, `value` then computes the variable.
struct Operand {
    var: &'static str,
    shape: Shape,
    guard: Vec<String>,
    value: Vec<String>,
}

enum Custom {
    Number(f64),
    /// Nanoseconds since the epoch.
    Instant(i128),
    /// Seconds from now, signed by the comparison.
    Offset(f64),
    Text(String),
    Texts(Vec<String>),
}

enum Second {
    Custom(Custom),
    Other(Operand),
}

/// The lines of a comparison: `guards` prove every value it reads is
/// present and of its shape, `body` derives what the tests compare.
#[derive(Default)]
struct Lines {
    guards: Vec<String>,
    body: Vec<String>,
}

/// The comparison: `lines` bind and derive, `tests` decide.
struct Comparison {
    lines: Lines,
    tests: Vec<String>,
    negate: bool,
}

fn property_name(property: &CatalogueProperty) -> String {
    format!("{} \"{}\"", property.app_name, property.name)
}

fn compile_rule(
    index: usize,
    rule: &DecodedRule,
    level: MediaLevel,
    catalogue: &Catalogue,
    known_usernames: &HashSet<String>,
) -> Result<CompiledRule, Failure> {
    let mut warnings = Vec::new();
    let first = lookup(index, rule.first, catalogue)?;
    let first_binding = bind_property(index, first, level)?;
    let first_name = property_name(first);
    if let Some(caveat) = first_binding.caveat {
        warnings.push(warning(
            codes::FACT_APPROXIMATE,
            format!("Rule {}: {first_name}: {caveat}.", index + 1),
            Some(index),
        ));
    }
    let username = if first_binding.source.is_per_user() {
        let Some(username) = rule.username.as_deref() else {
            return Err(Failure::rule(
                codes::RULE_USERNAME_MISSING,
                index,
                format!("{first_name} without a username"),
                format!("{first_name} needs a username."),
            ));
        };
        check_username(index, username, known_usernames, &mut warnings);
        Some(username.to_lowercase())
    } else {
        None
    };
    let unsupported = || {
        Failure::rule(
            codes::COMPARATOR_UNSUPPORTED,
            index,
            format!("\"{}\" on {first_name}", comparator_label(rule.action)),
            format!(
                "\"{}\" on {first_name} cannot be expressed faithfully.",
                comparator_label(rule.action)
            ),
        )
    };
    let first_operand = operand(&first_binding, level, "value", username.as_deref());

    let comparison = if matches!(rule.action, EXISTS | NOT_EXISTS) {
        if first_binding.source.is_per_user() {
            return Err(unsupported());
        }
        let mut guards = first_operand.guard;
        if matches!(
            first_operand.shape,
            Shape::Text | Shape::TextList | Shape::Instant | Shape::Day
        ) {
            guards.push("count(value) > 0".to_string());
        }
        Comparison {
            lines: Lines {
                guards,
                body: Vec::new(),
            },
            tests: Vec::new(),
            negate: rule.action == NOT_EXISTS,
        }
    } else {
        let second = if let Some(last) = rule.last {
            let other = lookup(index, last, catalogue)?;
            let other_binding = bind_property(index, other, level)?;
            if let Some(caveat) = other_binding.caveat {
                warnings.push(warning(
                    codes::FACT_APPROXIMATE,
                    format!("Rule {}: {}: {caveat}.", index + 1, property_name(other)),
                    Some(index),
                ));
            }
            if other_binding.source.is_per_user() && username.is_none() {
                let Some(name) = rule.username.as_deref() else {
                    return Err(Failure::rule(
                        codes::RULE_USERNAME_MISSING,
                        index,
                        format!("{} without a username", property_name(other)),
                        format!("{} needs a username.", property_name(other)),
                    ));
                };
                check_username(index, name, known_usernames, &mut warnings);
            }
            let other_username = rule.username.as_deref().map(str::to_lowercase);
            Second::Other(operand(
                &other_binding,
                level,
                "other",
                other_username.as_deref(),
            ))
        } else if let Some(custom) = &rule.custom {
            let decoded = decode_custom(custom, first.type_key == "1").map_err(|why| {
                Failure::rule(
                    codes::CUSTOM_VALUE_INVALID,
                    index,
                    format!("the value compared with {first_name}"),
                    format!("the value compared with {first_name} {why}."),
                )
            })?;
            if first_binding.usernames
                && !matches!(rule.action, CONTAINS_PARTIAL | NOT_CONTAINS_PARTIAL)
            {
                match &decoded {
                    Custom::Text(name) => {
                        check_username(index, name, known_usernames, &mut warnings)
                    }
                    Custom::Texts(names) => {
                        for name in names {
                            check_username(index, name, known_usernames, &mut warnings);
                        }
                    }
                    _ => {}
                }
            }
            Second::Custom(decoded)
        } else {
            return Err(Failure::rule(
                codes::RULE_SECOND_VALUE_MISSING,
                index,
                format!("rule {} has nothing to compare with", index + 1),
                format!("{first_name} has no value to compare with."),
            ));
        };
        if first.type_key == "1" && matches!(rule.action, EQUALS | NOT_EQUALS) {
            return Err(Failure::rule(
                codes::COMPARATOR_DATE_EQUALITY_UNSUPPORTED,
                index,
                format!("date equality on {first_name}"),
                format!(
                    "Maintainerr compares {first_name} by calendar day in its own time zone, which Scryer cannot reproduce."
                ),
            ));
        }
        compare(first_operand, rule.action, second).ok_or_else(unsupported)?
    };

    let name = format!("maintainerr_condition_{}", index + 1);
    let label = format!(
        "Rule {}: {} {}, {}",
        index + 1,
        first.app_name,
        first.human_name.as_deref().unwrap_or(&first.name),
        comparator_label(rule.action)
    );
    Ok(CompiledRule {
        rego: render_condition(&name, &comment(&label), comparison),
        warnings,
    })
}

fn check_username(
    index: usize,
    username: &str,
    known_usernames: &HashSet<String>,
    warnings: &mut Vec<MaintainerrDiagnostic>,
) {
    let lowered = username.to_lowercase();
    if lowered.is_empty() || known_usernames.contains(&lowered) {
        return;
    }
    warnings.push(warning(
        codes::USERNAME_UNKNOWN,
        format!(
            "Rule {}: no Scryer user is named \"{}\"; the rule only matches once one is.",
            index + 1,
            single_line(username)
        ),
        Some(index),
    ));
}

fn lookup(
    index: usize,
    (app, prop): (i64, i64),
    catalogue: &Catalogue,
) -> Result<&CatalogueProperty, Failure> {
    if !catalogue.applications.contains(&app) {
        return Err(Failure::rule(
            codes::APPLICATION_UNKNOWN,
            index,
            format!("an application Maintainerr does not list (application {app})"),
            format!(
                "application {app} is not in Maintainerr's property list; it is probably not configured there."
            ),
        ));
    }
    catalogue.properties.get(&(app, prop)).ok_or_else(|| {
        Failure::rule(
            codes::PROPERTY_UNKNOWN,
            index,
            format!("an unknown property (application {app}, property {prop})"),
            format!("application {app} property {prop} is not in Maintainerr's property list."),
        )
    })
}

fn bind_property(
    index: usize,
    property: &CatalogueProperty,
    level: MediaLevel,
) -> Result<Binding, Failure> {
    let name = property_name(property);
    bind::bind(&property.app_name, &property.name, &property.type_key, level).map_err(
        |refusal| match refusal {
            BindRefusal::Unmapped => Failure::rule(
                codes::PROPERTY_UNMAPPED,
                index,
                name.clone(),
                format!("{name} has no matching Scryer fact."),
            ),
            BindRefusal::Collection => Failure::rule(
                codes::PROPERTY_COLLECTION_UNSUPPORTED,
                index,
                format!("{name} (media-server collections, playlists and labels)"),
                format!(
                    "{name} reads media-server collections, playlists or labels, which Scryer does not track."
                ),
            ),
            BindRefusal::Level => Failure::rule(
                codes::PROPERTY_LEVEL_UNSUPPORTED,
                index,
                format!("{name} for {}", level.label()),
                format!("{name} has no matching Scryer fact for {}.", level.label()),
            ),
        },
    )
}

fn operand(
    binding: &Binding,
    level: MediaLevel,
    var: &'static str,
    username: Option<&str>,
) -> Operand {
    let (subject, facts) = match level {
        MediaLevel::Movie | MediaLevel::Show => ("input", "input.facts"),
        MediaLevel::Season => ("input.season", "input.season.facts"),
        MediaLevel::Episode => ("input.episode", "input.episode.facts"),
    };
    let type_guard = match binding.shape {
        Shape::Number => "is_number",
        Shape::Bool => "is_boolean",
        Shape::Instant | Shape::Day | Shape::Text => "is_string",
        Shape::TextList => "is_array",
    };
    let user = literal(username.unwrap_or_default());
    let (guard, value) = match binding.source {
        Source::Fact(fact) => (
            vec![
                format!("{var} := {facts}.{fact}"),
                format!("{type_guard}({var})"),
            ],
            Vec::new(),
        ),
        Source::Subject(field) => (
            vec![
                format!("{var} := {subject}.{field}"),
                format!("{type_guard}({var})"),
            ],
            Vec::new(),
        ),
        Source::MebibytesRounded(fact) => (
            vec![
                format!("{var}_bytes := {facts}.{fact}"),
                format!("is_number({var}_bytes)"),
            ],
            vec![format!("{var} := round({var}_bytes / 1048576)")],
        ),
        Source::Mebibytes(fact) => (
            vec![
                format!("{var}_bytes := {facts}.{fact}"),
                format!("is_number({var}_bytes)"),
            ],
            vec![format!("{var} := {var}_bytes / 1048576")],
        ),
        Source::UserPlayCount => (
            vec![
                format!("{var}_users := {facts}.watched_by_users"),
                format!("is_array({var}_users)"),
            ],
            vec![format!(
                "{var} := sum([w.play_count | some w in {var}_users; lower(w.username) == {user}])"
            )],
        ),
        Source::UserLastPlayedAt => (
            vec![
                format!("{var}_users := {facts}.watched_by_users"),
                format!("is_array({var}_users)"),
                format!(
                    "{var}_played := [w.last_played_at | some w in {var}_users; lower(w.username) == {user}; is_string(w.last_played_at)]"
                ),
                format!("count({var}_played) > 0"),
            ],
            vec![format!("{var} := {var}_played[0]")],
        ),
    };
    Operand {
        var,
        shape: binding.shape,
        guard,
        value,
    }
}

impl Operand {
    /// The guard and value lines, plus the derived variable the comparison
    /// reads: nanoseconds for a date, lower case for text.
    fn into_lines(self, out: &mut Lines) -> String {
        let var = self.var;
        out.guards.extend(self.guard);
        out.body.extend(self.value);
        let lines = &mut out.body;
        match self.shape {
            Shape::Instant => {
                lines.push(format!("{var}_ns := time.parse_rfc3339_ns({var})"));
                format!("{var}_ns")
            }
            Shape::Day => {
                lines.push(format!(
                    "{var}_ns := time.parse_rfc3339_ns(concat(\"\", [{var}, \"T00:00:00Z\"]))"
                ));
                format!("{var}_ns")
            }
            Shape::Text => {
                lines.push(format!("{var}_lower := lower({var})"));
                format!("{var}_lower")
            }
            Shape::TextList => {
                lines.push(format!("{var}_lower := [lower(x) | some x in {var}]"));
                format!("{var}_lower")
            }
            Shape::Number | Shape::Bool => var.to_string(),
        }
    }
}

/// The positive form of a negated comparison, and whether it was negated.
fn positive(action: i64) -> (i64, bool) {
    match action {
        NOT_EQUALS => (EQUALS, true),
        NOT_CONTAINS => (CONTAINS, true),
        NOT_CONTAINS_PARTIAL => (CONTAINS_PARTIAL, true),
        NOT_CONTAINS_ALL => (CONTAINS_ALL, true),
        COUNT_NOT_EQUALS => (COUNT_EQUALS, true),
        other => (other, false),
    }
}

fn compare(first: Operand, action: i64, second: Second) -> Option<Comparison> {
    let (action, negate) = positive(action);
    let shape = first.shape;
    let mut lines = Lines::default();
    let v = first.into_lines(&mut lines);
    let tests = match shape {
        Shape::Number => {
            let rhs = match second {
                Second::Custom(Custom::Number(n)) => number_literal(n),
                Second::Other(other) if other.shape == Shape::Number => {
                    other.into_lines(&mut lines)
                }
                _ => return None,
            };
            let op = match action {
                BIGGER => ">",
                SMALLER => "<",
                EQUALS => "==",
                _ => return None,
            };
            vec![format!("{v} {op} {rhs}")]
        }
        Shape::Bool => {
            let rhs = match second {
                Second::Custom(Custom::Number(n)) if n == 1.0 => "true".to_string(),
                Second::Custom(Custom::Number(n)) if n == 0.0 => "false".to_string(),
                Second::Other(other) if other.shape == Shape::Bool => other.into_lines(&mut lines),
                _ => return None,
            };
            if action != EQUALS {
                return None;
            }
            vec![format!("{v} == {rhs}")]
        }
        Shape::Instant | Shape::Day => {
            if negate {
                return None;
            }
            let needs_now = matches!(action, IN_LAST | IN_NEXT)
                || matches!(second, Second::Custom(Custom::Offset(_)));
            let rhs = match second {
                Second::Custom(Custom::Instant(ns)) => ns.to_string(),
                Second::Custom(Custom::Offset(seconds)) => {
                    let ns = seconds_to_nanos(seconds)?;
                    let sign = if matches!(action, IN_LAST | BEFORE) {
                        "-"
                    } else {
                        "+"
                    };
                    format!("now_ns {sign} {ns}")
                }
                Second::Other(other) if other.shape.is_date() => other.into_lines(&mut lines),
                _ => return None,
            };
            if needs_now {
                lines
                    .body
                    .push("now_ns := time.parse_rfc3339_ns(input.evaluation_time)".to_string());
            }
            match action {
                BEFORE => vec![format!("{v} <= {rhs}")],
                AFTER => vec![format!("{v} >= {rhs}")],
                BIGGER => vec![format!("{v} > {rhs}")],
                SMALLER => vec![format!("{v} < {rhs}")],
                IN_LAST => vec![format!("{v} >= {rhs}"), format!("{v} <= now_ns")],
                IN_NEXT => vec![format!("{v} <= {rhs}"), format!("{v} >= now_ns")],
                _ => return None,
            }
        }
        Shape::Text => compare_text(&v, action, second, &mut lines)?,
        Shape::TextList => compare_list(&v, action, second, &mut lines)?,
    };
    Some(Comparison {
        lines,
        tests,
        negate,
    })
}

fn compare_text(v: &str, action: i64, second: Second, lines: &mut Lines) -> Option<Vec<String>> {
    Some(match (action, second) {
        (EQUALS, Second::Custom(Custom::Text(text))) => vec![format!("{v} == {}", literal(&text))],
        (EQUALS | CONTAINS | CONTAINS_ALL, Second::Other(other)) if other.shape == Shape::Text => {
            let o = other.into_lines(lines);
            if action == EQUALS {
                vec![format!("{v} == {o}")]
            } else {
                vec![format!("contains({v}, {o})")]
            }
        }
        (CONTAINS | CONTAINS_ALL, Second::Custom(Custom::Text(text))) => {
            vec![format!("contains({v}, {})", literal(&text))]
        }
        (CONTAINS_PARTIAL, Second::Custom(Custom::Text(text))) => {
            if text.is_empty() {
                vec![format!("{v} == \"\"")]
            } else {
                vec![format!("contains({v}, {})", literal(&text))]
            }
        }
        (CONTAINS, Second::Custom(Custom::Texts(texts))) => vec![format!(
            "count([c | some c in {}; contains({v}, c)]) > 0",
            list_literal(&texts)
        )],
        (CONTAINS_PARTIAL, Second::Custom(Custom::Texts(texts)))
            if texts.iter().all(|text| !text.is_empty()) =>
        {
            vec![format!(
                "count([c | some c in {}; contains({v}, c)]) > 0",
                list_literal(&texts)
            )]
        }
        (CONTAINS_ALL, Second::Custom(Custom::Texts(texts))) => vec![format!(
            "count([c | some c in {}; not contains({v}, c)]) == 0",
            list_literal(&texts)
        )],
        _ => return None,
    })
}

fn compare_list(v: &str, action: i64, second: Second, lines: &mut Lines) -> Option<Vec<String>> {
    Some(match (action, second) {
        (CONTAINS | CONTAINS_ALL, Second::Custom(Custom::Text(text))) => {
            vec![format!("{} in {v}", literal(&text))]
        }
        (CONTAINS, Second::Custom(Custom::Texts(texts))) => vec![format!(
            "count([c | some c in {}; c in {v}]) > 0",
            list_literal(&texts)
        )],
        (CONTAINS_ALL, Second::Custom(Custom::Texts(texts))) => vec![format!(
            "count([c | some c in {}; not c in {v}]) == 0",
            list_literal(&texts)
        )],
        (CONTAINS_PARTIAL, Second::Custom(Custom::Text(text))) => {
            if text.is_empty() {
                vec![format!("\"\" in {v}")]
            } else {
                vec![format!(
                    "count([x | some x in {v}; contains(x, {})]) > 0",
                    literal(&text)
                )]
            }
        }
        (CONTAINS_PARTIAL, Second::Custom(Custom::Texts(texts)))
            if texts.iter().all(|text| !text.is_empty()) =>
        {
            vec![format!(
                "count([x | some x in {v}; some c in {}; contains(x, c)]) > 0",
                list_literal(&texts)
            )]
        }
        (EQUALS, Second::Custom(Custom::Text(text))) => list_equals(v, &[text]),
        (EQUALS, Second::Custom(Custom::Texts(texts))) => list_equals(v, &texts),
        (COUNT_EQUALS | COUNT_BIGGER | COUNT_SMALLER, Second::Custom(custom)) => {
            let rhs = match custom {
                Custom::Number(n) => number_literal(n),
                Custom::Texts(texts) => texts.len().to_string(),
                _ => return None,
            };
            vec![format!("count({v}) {} {rhs}", count_operator(action))]
        }
        (_, Second::Other(other)) if other.shape == Shape::Text => {
            let o = other.into_lines(lines);
            match action {
                CONTAINS | CONTAINS_ALL => vec![format!("{o} in {v}")],
                EQUALS => vec![format!("count({v}) == 1"), format!("{v}[0] == {o}")],
                _ => return None,
            }
        }
        (_, Second::Other(other)) if other.shape == Shape::TextList => {
            let o = other.into_lines(lines);
            match action {
                CONTAINS => vec![format!("count([c | some c in {o}; c in {v}]) > 0")],
                CONTAINS_ALL => vec![
                    format!("count({o}) > 0"),
                    format!("count([c | some c in {o}; not c in {v}]) == 0"),
                ],
                EQUALS => vec![
                    format!("count({v}) == count({o})"),
                    format!("count([x | some x in {v}; not x in {o}]) == 0"),
                ],
                COUNT_EQUALS | COUNT_BIGGER | COUNT_SMALLER => {
                    vec![format!("count({v}) {} count({o})", count_operator(action))]
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn count_operator(action: i64) -> &'static str {
    match action {
        COUNT_BIGGER => ">",
        COUNT_SMALLER => "<",
        _ => "==",
    }
}

/// Maintainerr's list equality: the same length, and every element of the
/// value among the expected ones.
fn list_equals(v: &str, expected: &[String]) -> Vec<String> {
    vec![
        format!("count({v}) == {}", expected.len()),
        format!(
            "count([x | some x in {v}; not x in {}]) == 0",
            list_literal(expected)
        ),
    ]
}

fn seconds_to_nanos(seconds: f64) -> Option<i128> {
    if !seconds.is_finite() || seconds.abs() > MAX_ABS_SECONDS {
        return None;
    }
    // Maintainerr builds a JavaScript Date, which keeps whole milliseconds.
    Some((seconds * 1000.0).trunc() as i128 * NANOS_PER_MILLI)
}

/// Reads a custom value the way Maintainerr's comparator reads it.
/// `first_is_date` is whether the compared property is a date property.
fn decode_custom(custom: &CustomValue, first_is_date: bool) -> Result<Custom, &'static str> {
    let value = custom.value.as_str();
    match custom.rule_type_id {
        CUSTOM_DATE => {
            let ns = if value.contains('-') {
                parse_date(value).ok_or("is not a date Scryer can read the same way")?
            } else {
                seconds_to_nanos(js_number(value).ok_or("is not a timestamp")?)
                    .ok_or("is out of range")?
            };
            Ok(Custom::Instant(ns))
        }
        CUSTOM_TEXT | CUSTOM_TEXT_LIST => {
            if let Ok(serde_json::Value::Array(items)) =
                serde_json::from_str::<serde_json::Value>(value)
            {
                if items.is_empty() {
                    return Err("is an empty list");
                }
                let texts = items
                    .iter()
                    .map(|item| item.as_str().map(str::to_lowercase))
                    .collect::<Option<Vec<_>>>()
                    .ok_or("is a list of something other than text")?;
                Ok(Custom::Texts(texts))
            } else {
                Ok(Custom::Text(value.to_lowercase()))
            }
        }
        CUSTOM_NUMBER | CUSTOM_BOOL => {
            let number = js_number(value).ok_or("is not a number")?;
            if first_is_date && custom.rule_type_id == CUSTOM_NUMBER {
                Ok(Custom::Offset(number))
            } else {
                Ok(Custom::Number(number))
            }
        }
        _ => Err("has an unknown type"),
    }
}

/// JavaScript's unary plus on a string, for the decimal forms Rust also
/// reads; anything else is refused rather than guessed.
fn js_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    if trimmed
        .chars()
        .any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
    {
        return None;
    }
    trimmed
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}

/// A date string as JavaScript's `new Date(text)` reads it, in nanoseconds,
/// for the forms that do not depend on the local time zone.
fn parse_date(text: &str) -> Option<i128> {
    let text = text.trim();
    let millis = if let Ok(day) = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        if text.len() != 10 {
            return None;
        }
        day.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis()
    } else {
        chrono::DateTime::parse_from_rfc3339(text)
            .ok()?
            .timestamp_millis()
    };
    let ns = millis as i128 * NANOS_PER_MILLI;
    (ns.abs() <= (MAX_ABS_SECONDS as i128) * 1_000_000_000).then_some(ns)
}

// ── Rendering ──────────────────────────────────────────────────────────────

fn render_condition(name: &str, label: &str, comparison: Comparison) -> String {
    let Comparison {
        lines: Lines { guards, body },
        tests,
        negate,
    } = comparison;
    let mut out = format!("# {label}\n");
    if !negate {
        out.push_str(&rule_block(name, guards.iter().chain(&body).chain(&tests)));
        return out;
    }
    // A negated comparison still needs every value it reads: Maintainerr
    // never matches on a value that is missing.
    let helper = if tests.is_empty() {
        // NOT_EXISTS: no value of the property's shape is present.
        out.push_str(&rule_block(&format!("{name}_present"), guards.iter()));
        out.push('\n');
        out.push_str(&format!("{name} if not {name}_present\n"));
        return out;
    } else {
        format!("{name}_holds")
    };
    out.push_str(&rule_block(
        &helper,
        guards.iter().chain(&body).chain(&tests),
    ));
    out.push('\n');
    let negated = format!("not {helper}");
    out.push_str(&rule_block(
        name,
        guards.iter().chain(std::iter::once(&negated)),
    ));
    out
}

fn rule_block<'a>(name: &str, lines: impl Iterator<Item = &'a String>) -> String {
    let mut out = format!("{name} if {{\n");
    for line in lines {
        out.push('\t');
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// A rule with one definition per body; each body is a conjunction of atoms.
fn render_bodies(name: &str, bodies: &[Vec<String>]) -> String {
    bodies
        .iter()
        .map(|body| {
            if body.len() == 1 {
                format!("{name} if {}\n", body[0])
            } else {
                rule_block(name, body.iter())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Folds `(operator, atom)` pairs left to right, as Maintainerr folds rules
/// onto a running set: OR adds the atom's matches, AND keeps only those the
/// atom also matches. The result is a disjunction of conjunctions.
fn fold(
    items: Vec<(Operator, String)>,
    prefix: &str,
    helpers: &mut Vec<String>,
) -> Vec<Vec<String>> {
    let mut bodies: Vec<Vec<String>> = Vec::new();
    let mut steps = 0;
    for (operator, atom) in items {
        match operator {
            Operator::Or => bodies.push(vec![atom]),
            Operator::And => {
                if bodies.is_empty() {
                    // Nothing matched so far, and AND cannot add anything.
                    continue;
                }
                if bodies.len() > 1 {
                    steps += 1;
                    let step = format!("{prefix}_step_{steps}");
                    helpers.push(render_bodies(&step, &bodies));
                    bodies = vec![vec![step]];
                }
                bodies[0].push(atom);
            }
        }
    }
    bodies
}

struct Section {
    join: Operator,
    items: Vec<(Operator, String)>,
}

/// Groups the rules into sections exactly as Maintainerr's comparator walks
/// them: the very first rule's operator is ignored, a change of section
/// number starts a new section whose join with the result so far is its
/// first rule's operator (AND only when explicitly AND), and that first rule
/// then starts the section afresh.
fn sections(rules: &[DecodedRule]) -> Vec<Section> {
    let mut current = 0;
    let mut out = vec![Section {
        join: Operator::Or,
        items: Vec::new(),
    }];
    for (index, rule) in rules.iter().enumerate() {
        let operator = if index == 0 { None } else { rule.operator };
        let atom = format!("maintainerr_condition_{}", index + 1);
        if rule.section == current {
            let section = out.last_mut().expect("there is always a section");
            let operator = if section.items.is_empty() {
                Operator::Or
            } else {
                operator.unwrap_or(Operator::Or)
            };
            section.items.push((operator, atom));
        } else {
            out.push(Section {
                join: operator.unwrap_or(Operator::Or),
                items: vec![(Operator::Or, atom)],
            });
            current = rule.section;
        }
    }
    out.retain(|section| !section.items.is_empty());
    out
}

fn render(
    group: &DecodedGroup,
    display: &str,
    level: MediaLevel,
    rules: &[DecodedRule],
    conditions: &[String],
    plan: &ActionPlan,
) -> Result<String, Failure> {
    let mut helpers = Vec::new();
    let mut section_atoms = Vec::new();
    let sections = sections(rules);
    let numbered = sections.len() > 1;
    for (number, section) in sections.into_iter().enumerate() {
        let name = format!("maintainerr_section_{}", number + 1);
        let bodies = fold(section.items, &name, &mut helpers);
        let atom = match bodies.as_slice() {
            [] => None,
            [body] if body.len() == 1 => Some(body[0].clone()),
            _ if !numbered => {
                // A single section is the whole match.
                section_atoms.push((section.join, bodies));
                continue;
            }
            _ => {
                helpers.push(render_bodies(&name, &bodies));
                Some(name)
            }
        };
        if let Some(atom) = atom {
            section_atoms.push((section.join, vec![vec![atom]]));
        } else {
            section_atoms.push((section.join, Vec::new()));
        }
    }
    let matches = if !numbered {
        section_atoms
            .pop()
            .map(|(_, bodies)| bodies)
            .unwrap_or_default()
    } else {
        let items = section_atoms
            .into_iter()
            .filter_map(|(join, bodies)| bodies.into_iter().next().map(|body| (join, body)))
            .map(|(join, body)| (join, body.into_iter().next().unwrap_or_default()))
            .filter(|(_, atom)| !atom.is_empty())
            .collect();
        fold(items, "maintainerr_matches", &mut helpers)
    };
    if matches.is_empty() {
        return Err(Failure::group(
            codes::GROUP_INVALID,
            "the rules can never match",
            "The group's rules can never match anything.",
        ));
    }

    let facet = match level {
        MediaLevel::Movie => "input.subject.facet == \"movie\"",
        _ => "input.subject.facet in {\"series\", \"anime\"}",
    };
    let mut out = String::new();
    out.push_str(&format!(
        "# {}\n",
        comment(&format!(
            "Imported from Maintainerr rule group {}: {display}",
            group.id
        ))
    ));
    out.push_str(&format!(
        "# Media: {}. Maintainerr action: {}.\n\n",
        level.label(),
        plan.label
    ));
    for condition in conditions {
        out.push_str(condition);
        out.push('\n');
    }
    for helper in &helpers {
        out.push_str(helper);
        out.push('\n');
    }
    out.push_str(&render_bodies("maintainerr_matches", &matches));
    out.push('\n');
    out.push_str(&rule_block(
        "maintainerr_applies",
        [facet.to_string(), "maintainerr_matches".to_string()].iter(),
    ));
    out.push('\n');
    for (head, value) in &plan.heads {
        match value {
            None => out.push_str(&format!("{} if maintainerr_applies\n", head.rule_name())),
            Some(value) => out.push_str(&format!(
                "{} := {} if maintainerr_applies\n",
                head.rule_name(),
                literal(value)
            )),
        }
    }
    out.push_str("\nreasons contains \"maintainerr_rule_group_matched\" if maintainerr_applies\n");
    Ok(out)
}

fn exceeds_limits(source: &str) -> Option<Failure> {
    let limits = RuntimeLimits::lifecycle_defaults();
    let too_large = source.len() > limits.max_policy_bytes.get()
        || source.lines().count() > limits.max_policy_lines.get()
        || source
            .lines()
            .any(|line| line.len() > limits.max_policy_col.get() as usize);
    too_large.then(|| {
        Failure::group(
            codes::SOURCE_TOO_LARGE,
            "the translated rule is too large",
            "The translated rule exceeds the size limits of a lifecycle rule.",
        )
    })
}

/// Validates a generated source as a saved rule is validated, and reports
/// whether it reads a fact about identifiable people.
fn check_source(source: &str) -> Result<bool, Failure> {
    let rewritten = rewrite_package_declaration(source, VALIDATION_RULE_ID);
    let invalid = |detail: String| {
        Failure::group(
            codes::SOURCE_INVALID,
            "the translated rule did not validate",
            format!("The translated rule did not validate: {detail}"),
        )
    };
    let validation = validate_lifecycle_rule(&rewritten, VALIDATION_RULE_ID)
        .map_err(|error| invalid(error.to_string()))?;
    if !validation.valid {
        return Err(invalid(
            validation
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .chain(validation.errors.iter().cloned())
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let referenced = lifecycle_referenced_facts(&rewritten, VALIDATION_RULE_ID)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(PERSON_TARGETED_LIFECYCLE_FACTS
        .iter()
        .any(|fact| referenced.contains(*fact)))
}
