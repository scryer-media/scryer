//! Sonarr and Radarr custom formats imported as scoring rule sets: one
//! disabled rule set per custom format, scored from the instance's quality
//! profiles.
//!
//! A format every profile scores alike keeps that one score. When profiles
//! disagree, the score follows `input.profile.name` if every profile of the
//! instance has a Scryer quality profile of exactly the same name; otherwise
//! the rule scores 0, is named for review, and carries the per-profile table
//! commented out for the operator to adapt.

use crate::arr_custom_format::{
    ArrDiagnostic, ArrDiagnosticSeverity, ArrFormatScoring, ArrFormatStatus, ArrSourceApplication,
    arr_source_facets, codes as diagnostic_codes, translate_arr_custom_format,
};
use crate::rule_import::{
    FetchedArrRules, MAX_ITEMS, PlannedRuleImportItem, PlannedRuleImportTranslation,
    RuleImportFact, RuleImportItemState, RuleImportPlanContext, RuleImportSaveContext,
    RuleImportSaveOutcome, StagedRuleImportRuleSet, refusal,
};
use crate::{AppError, AppResult, NewRuleSet};
use scryer_domain::MediaFacet;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub mod reasons {
    /// A custom format uses a specification Scryer cannot translate, so no
    /// rule was saved; the item's facts carry the translator's diagnostics.
    pub const FORMAT_UNTRANSLATABLE: &str = "RULE_IMPORT_FORMAT_UNTRANSLATABLE";
    /// A custom format could not be read at all, so no rule was saved.
    pub const FORMAT_UNREADABLE: &str = "RULE_IMPORT_FORMAT_UNREADABLE";
    /// Quality profiles score the format differently and at least one cannot
    /// be matched to a single Scryer quality profile of the same name, because
    /// none exists or the source repeats the name with another score; the
    /// rule scores 0 until reviewed.
    pub const PROFILE_SCORES_UNMATCHED: &str = "RULE_IMPORT_PROFILE_SCORES_UNMATCHED";
    /// Quality profiles score the format differently; the rule scores by
    /// Scryer quality profile name.
    pub const PROFILE_SCORES_APPLIED: &str = "RULE_IMPORT_PROFILE_SCORES_APPLIED";
    /// Every quality profile scores the format 0, so the rule scores 0.
    pub const FORMAT_UNSCORED: &str = "RULE_IMPORT_FORMAT_UNSCORED";
    pub const ALL: &[&str] = &[
        FORMAT_UNTRANSLATABLE,
        FORMAT_UNREADABLE,
        PROFILE_SCORES_UNMATCHED,
        PROFILE_SCORES_APPLIED,
        FORMAT_UNSCORED,
    ];
}

/// Longest source version kept in a rule's description.
const MAX_VERSION_CHARS: usize = 64;

/// What one custom format saves as.
#[derive(Debug)]
pub enum PlannedArrScoringRule {
    Save(ArrScoringRuleDraft),
    /// Nothing is saved for the format.
    Failed {
        reason: &'static str,
        facts: Vec<RuleImportFact>,
    },
}

#[derive(Debug)]
pub struct ArrScoringRuleDraft {
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub applied_facets: Vec<MediaFacet>,
    /// Every name this format saves under, with and without the review
    /// prefix: a rule set holding any of them means the format was imported.
    pub claimed_names: Vec<String>,
    /// `Saved` or `SavedNeedsReview`.
    pub state: RuleImportItemState,
    pub reason: Option<&'static str>,
    pub facts: Vec<RuleImportFact>,
}

fn label(application: ArrSourceApplication) -> &'static str {
    match application {
        ArrSourceApplication::Radarr => "Radarr",
        ArrSourceApplication::Sonarr => "Sonarr",
    }
}

/// Text from the source made safe to store and to print in a comment: control
/// characters and line or paragraph separators become spaces, and runs of
/// whitespace collapse to one space.
fn clean_text(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn diagnostic_fact(diagnostic: ArrDiagnostic) -> RuleImportFact {
    let severity = match diagnostic.severity {
        ArrDiagnosticSeverity::Error => "ERROR",
        ArrDiagnosticSeverity::Warning => "WARNING",
        ArrDiagnosticSeverity::Info => "INFO",
    };
    RuleImportFact::new(diagnostic.code, diagnostic.message).with_details(json!({
        "severity": severity,
        "location": diagnostic.location,
        "specificationIndex": diagnostic.specification_index,
    }))
}

fn scores_details(scores: &[(String, i32)]) -> Value {
    Value::Array(
        scores
            .iter()
            .map(|(profile, score)| json!({ "profile": profile, "score": score }))
            .collect(),
    )
}

/// How the instance's profiles score one format. `scores` carry the source's
/// profile names, for the report; `table` carries the names the rule looks
/// up in `input.profile.name`: the stored name of the matching Scryer
/// profile, or the source's name where there is none.
enum Scoring {
    Literal(i32),
    /// Profiles disagree and each has a Scryer profile of the same name.
    Matched {
        scores: Vec<(String, i32)>,
        table: Vec<(String, i32)>,
    },
    /// Profiles disagree and `unmatched` name no Scryer profile, or the
    /// source repeats the `duplicates` names with different scores.
    Unmatched {
        scores: Vec<(String, i32)>,
        table: Vec<(String, i32)>,
        unmatched: Vec<String>,
        duplicates: Vec<String>,
    },
}

/// Profile scores for one format, in the instance's profile order. Each
/// profile is its source name with the stored name of the Scryer profile it
/// matches. A profile that does not list the format scores 0. A name the
/// instance repeats keeps its first score; a repeat with a different score
/// is reported as a duplicate, since one name cannot carry two scores.
fn scoring(profiles: &[(String, Option<String>)], listed: &HashMap<usize, i32>) -> Scoring {
    let score_of = |index: usize| listed.get(&index).copied().unwrap_or(0);
    let first = if profiles.is_empty() { 0 } else { score_of(0) };
    if (0..profiles.len()).all(|index| score_of(index) == first) {
        return Scoring::Literal(first);
    }
    let mut scores: Vec<(String, i32)> = Vec::with_capacity(profiles.len());
    let mut table: Vec<(String, i32)> = Vec::with_capacity(profiles.len());
    let mut seen: HashMap<&str, i32> = HashMap::with_capacity(profiles.len());
    let mut unmatched: Vec<String> = Vec::new();
    let mut duplicates: Vec<String> = Vec::new();
    let mut duplicates_seen: HashSet<&str> = HashSet::new();
    for (index, (name, scryer_name)) in profiles.iter().enumerate() {
        let score = score_of(index);
        match seen.get(name.as_str()) {
            Some(kept) => {
                if *kept != score && duplicates_seen.insert(name) {
                    duplicates.push(name.clone());
                }
            }
            None => {
                seen.insert(name, score);
                scores.push((name.clone(), score));
                table.push((scryer_name.clone().unwrap_or_else(|| name.clone()), score));
                if scryer_name.is_none() {
                    unmatched.push(name.clone());
                }
            }
        }
    }
    if unmatched.is_empty() && duplicates.is_empty() {
        Scoring::Matched { scores, table }
    } else {
        Scoring::Unmatched {
            scores,
            table,
            unmatched,
            duplicates,
        }
    }
}

/// Plans one disabled scoring rule set per custom format.
pub fn plan_arr_rule_import(
    application: ArrSourceApplication,
    fetched: &FetchedArrRules,
    context: &RuleImportPlanContext,
) -> AppResult<Vec<PlannedRuleImportItem>> {
    if fetched.custom_formats.len() > MAX_ITEMS {
        return Err(AppError::validation_refused(
            refusal::TOO_MANY_ITEMS,
            "the source holds more objects than one import accepts",
        ));
    }
    let label = label(application);
    let version: String = clean_text(&fetched.version)
        .chars()
        .take(MAX_VERSION_CHARS)
        .collect();
    let description = if version.is_empty() {
        format!("Imported from {label}.")
    } else {
        format!("Imported from {label} {version}.")
    };
    let notes = vec![format!(
        "Imported from {label} {version}. Saved disabled; enable it once reviewed."
    )];
    let review_notes = vec![
        format!(
            "REVIEW: the {label} quality profiles score this format differently, and some of them cannot be matched to one Scryer quality profile with the same name."
        ),
        "This rule scores 0 until it is reviewed. Make the section below fit your Scryer quality profiles, then use it in place of the score_entry rule above.".to_string(),
    ];
    let applied_facets: Vec<MediaFacet> = arr_source_facets(application)
        .iter()
        .filter_map(|facet| MediaFacet::parse(facet))
        .collect();

    // Scryer profiles whose stored name has surrounding whitespace, by their
    // trimmed name; `None` when two of them trim to the same name.
    let mut padded: HashMap<&str, Option<&str>> = HashMap::new();
    for stored in &context.quality_profile_names {
        let trimmed = stored.trim();
        if trimmed != stored {
            padded
                .entry(trimmed)
                .and_modify(|kept| *kept = None)
                .or_insert(Some(stored));
        }
    }
    // Profile names, each with the stored name of the Scryer profile whose
    // name matches it exactly once both are trimmed.
    let profiles: Vec<(String, Option<String>)> = fetched
        .quality_profiles
        .iter()
        .map(|profile| {
            let name = profile.name.trim();
            let scryer_name = if context.quality_profile_names.contains(name) {
                Some(name.to_string())
            } else {
                padded.get(name).copied().flatten().map(str::to_string)
            };
            (name.to_string(), scryer_name)
        })
        .collect();
    // Format id -> (profile index -> score), built once for every format.
    let mut listed: HashMap<i64, HashMap<usize, i64>> = HashMap::new();
    for (index, profile) in fetched.quality_profiles.iter().enumerate() {
        for item in &profile.format_items {
            listed
                .entry(item.format)
                .or_default()
                .insert(index, item.score);
        }
    }
    let empty = HashMap::new();

    let mut planned = Vec::with_capacity(fetched.custom_formats.len());
    for (index, raw) in fetched.custom_formats.iter().enumerate() {
        let format_id = raw.get("id").and_then(Value::as_i64);
        let key = match format_id {
            Some(id) => format!("customformat:{id}"),
            None => format!("customformat:#{index}"),
        };
        let raw_name = raw
            .get("name")
            .and_then(Value::as_str)
            .map(clean_text)
            .filter(|name| !name.is_empty());
        let by_profile = format_id.and_then(|id| listed.get(&id)).unwrap_or(&empty);
        let mut out_of_range = false;
        let by_profile: HashMap<usize, i32> = by_profile
            .iter()
            .filter_map(|(profile, score)| match i32::try_from(*score) {
                Ok(score) => Some((*profile, score)),
                Err(_) => {
                    out_of_range = true;
                    None
                }
            })
            .collect();
        let source_name = raw_name.clone().unwrap_or_else(|| key.clone());
        if out_of_range {
            planned.push(PlannedRuleImportItem {
                key,
                source_name,
                translation: PlannedRuleImportTranslation::ArrScoringRule(
                    PlannedArrScoringRule::Failed {
                        reason: reasons::FORMAT_UNTRANSLATABLE,
                        facts: vec![RuleImportFact::new(
                            diagnostic_codes::SCORE_INVALID,
                            "a quality profile scores this format outside the signed 32-bit range",
                        )],
                    },
                ),
            });
            continue;
        }
        let scoring = scoring(&profiles, &by_profile);
        let (format_scoring, state, reason, facts) = match &scoring {
            Scoring::Literal(0) => (
                ArrFormatScoring::Literal(0),
                RuleImportItemState::Saved,
                None,
                vec![RuleImportFact::new(
                    reasons::FORMAT_UNSCORED,
                    "every quality profile scores this format 0",
                )],
            ),
            Scoring::Literal(score) => (
                ArrFormatScoring::Literal(*score),
                RuleImportItemState::Saved,
                None,
                Vec::new(),
            ),
            Scoring::Matched { scores, table } => (
                ArrFormatScoring::ProfileTable(
                    table
                        .iter()
                        .filter(|(_, score)| *score != 0)
                        .cloned()
                        .collect(),
                ),
                RuleImportItemState::Saved,
                None,
                vec![
                    RuleImportFact::new(
                        reasons::PROFILE_SCORES_APPLIED,
                        "the score follows the Scryer quality profile of the same name",
                    )
                    .with_details(json!({ "scores": scores_details(scores) })),
                ],
            ),
            Scoring::Unmatched {
                scores,
                table,
                unmatched,
                duplicates,
            } => (
                ArrFormatScoring::ReviewTable(table.clone()),
                RuleImportItemState::SavedNeedsReview,
                Some(reasons::PROFILE_SCORES_UNMATCHED),
                vec![
                    RuleImportFact::new(
                        reasons::PROFILE_SCORES_UNMATCHED,
                        "some quality profiles cannot be matched to one Scryer quality profile with the same name",
                    )
                    .with_details(json!({
                        "unmatchedProfiles": unmatched,
                        "duplicateProfiles": duplicates,
                        "scores": scores_details(scores),
                    })),
                ],
            ),
        };
        let translated =
            translate_arr_custom_format(application, raw, &format_scoring, &notes, &review_notes);
        let source_name = translated
            .name
            .as_deref()
            .map(clean_text)
            .filter(|name| !name.is_empty())
            .unwrap_or(source_name);
        let rule = match (translated.status, translated.rego_source) {
            (Some(ArrFormatStatus::Translated), Some(rego_source)) => {
                let plain = format!("({label}) {source_name}");
                let review = format!("({label}) REVIEW - {source_name}");
                let (name, other) = if state == RuleImportItemState::SavedNeedsReview {
                    (review, plain)
                } else {
                    (plain, review)
                };
                PlannedArrScoringRule::Save(ArrScoringRuleDraft {
                    claimed_names: vec![name.clone(), other],
                    name,
                    description: description.clone(),
                    rego_source,
                    applied_facets: applied_facets.clone(),
                    state,
                    reason,
                    facts,
                })
            }
            (status, _) => PlannedArrScoringRule::Failed {
                reason: if status.is_none() {
                    reasons::FORMAT_UNREADABLE
                } else {
                    reasons::FORMAT_UNTRANSLATABLE
                },
                facts: translated
                    .diagnostics
                    .into_iter()
                    .map(diagnostic_fact)
                    .collect(),
            },
        };
        planned.push(PlannedRuleImportItem {
            key,
            source_name,
            translation: PlannedRuleImportTranslation::ArrScoringRule(rule),
        });
    }
    Ok(planned)
}

/// Decides one planned format against the run's existing names: a format is
/// already imported when a rule set holds either of its names, with or
/// without the review prefix. A rule to write is staged, and stays pending
/// until the run's batch is written, which checks the names again.
pub(crate) fn stage_arr_scoring_rule(
    context: &mut RuleImportSaveContext,
    item_index: usize,
    rule: PlannedArrScoringRule,
) -> RuleImportSaveOutcome {
    match rule {
        PlannedArrScoringRule::Failed { reason, facts } => RuleImportSaveOutcome {
            state: RuleImportItemState::Failed,
            saved_name: None,
            rule_set_id: None,
            reason: Some(reason),
            facts,
        },
        PlannedArrScoringRule::Save(draft) => {
            if let Some(taken) = draft
                .claimed_names
                .iter()
                .find(|name| context.existing_names.contains(*name))
            {
                return RuleImportSaveOutcome {
                    state: RuleImportItemState::SkippedExists,
                    saved_name: Some(taken.clone()),
                    rule_set_id: None,
                    reason: Some(refusal::ALREADY_EXISTS),
                    facts: draft.facts,
                };
            }
            context
                .existing_names
                .extend(draft.claimed_names.iter().cloned());
            context.staged_rule_sets.push(StagedRuleImportRuleSet {
                item_index,
                state: draft.state,
                claimed_names: draft.claimed_names,
                rule_set: NewRuleSet {
                    name: draft.name.clone(),
                    description: draft.description,
                    rego_source: draft.rego_source,
                    applied_facets: draft.applied_facets,
                    priority: 0,
                    enabled: false,
                },
            });
            RuleImportSaveOutcome {
                state: RuleImportItemState::Pending,
                saved_name: Some(draft.name),
                rule_set_id: None,
                reason: draft.reason,
                facts: draft.facts,
            }
        }
    }
}

#[cfg(test)]
#[path = "rule_import_arr_tests.rs"]
mod tests;
