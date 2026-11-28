//! Heads a request rule declares in its own source.
//!
//! The three votes decide a request: `approve`, `deny` and `manual`. Five
//! shaping heads change what an approval writes, each replacing what the
//! requester asked for: `set_quality_profile`, `set_lease_days` (`0` keeps the
//! title forever), `set_monitor`, `set_required_audio_languages` and
//! `set_preferred_audio_languages`. `add_tags` tags the created title and
//! `notify` sends a notification. `reasons` carries stable codes. Any other
//! rule name is an author helper and means nothing to the host.
//!
//! Two more votes decide a bad-file report instead of a request: `mark_bad`
//! marks the reported file bad and `reject_report` rejects the report. A rule
//! declares report heads or request heads, never both; `notify` and `reasons`
//! go with either. No head recycles a file or starts a search.
//!
//! [`REQUEST_HEADS`] is the one description of those heads. The wrapper, the
//! runtime decode, the static scan, the declared-actions summary and the
//! builder schema all read it, so a new head is one entry there plus the code
//! that applies its value.
//!
//! Every valued head takes a plain literal, so the host can check every value
//! a rule could ever produce before it runs.

use std::collections::{BTreeMap, BTreeSet};

use regorus::Value;
use regorus::unstable::{Expr, Module};
use serde::{Deserialize, Serialize};

use crate::RulesError;
use crate::diagnostic_codes;
use crate::policy::decode::{
    MAX_REASON_CODE_LEN, MAX_REASON_CODES, MAX_TAG_LEN, MAX_TAGS, TAG_PATTERN, check_tag,
};
use crate::rule_builder_schema::{
    RuleBuilderOutputDescriptor, RuleBuilderOutputRule, RuleBuilderOutputRuleKind,
    RuleBuilderSchema,
};
use crate::rule_scan::{
    RuleLiteral, RuleScanIssue, foreign_data_references, head_defined_as_function,
    is_function_rule, issue, plain_value_exprs, referenced_names, rule_refr, source_position,
    string_literal,
};
use crate::runtime::RuntimeLimits;
use crate::validation::{parse_module, rule_head_name};

/// The longest lease a rule may set, in days. `0` is forever.
pub const MAX_LEASE_DAYS: i64 = 3650;
/// The most audio languages one language head may list.
pub const MAX_AUDIO_LANGUAGES: usize = 16;
/// The longest audio language code a rule may write.
pub const MAX_AUDIO_LANGUAGE_LEN: usize = 32;
/// The characters an audio language code a rule writes may contain.
pub const AUDIO_LANGUAGE_PATTERN: &str = "[A-Za-z_-]+";
/// The longest notification text a rule may write, in bytes.
pub const MAX_NOTIFY_LEN: usize = 500;

/// The rule family the builder schema describes.
pub const REQUEST_RULE_FAMILY: &str = "request";
/// The helper rule the builder's templates refer to.
pub const BUILDER_HELPER_RULE: &str = "rule_applies";

// ── Heads ───────────────────────────────────────────────────────────────────

/// One head a request rule may define, other than `reasons`.
///
/// The discriminant is the head's index in [`REQUEST_HEADS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestActionHead {
    Approve,
    Deny,
    Manual,
    SetQualityProfile,
    SetLeaseDays,
    SetMonitor,
    SetRequiredAudioLanguages,
    SetPreferredAudioLanguages,
    AddTags,
    Notify,
    MarkBad,
    RejectReport,
}

/// What a head does to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestHeadRole {
    /// Approve, deny or send to manual review.
    Vote,
    /// Replace a value the approval writes.
    Shaping,
    /// Tag the title the request creates.
    Tags,
    /// Send a notification.
    Notify,
    /// Mark a reported file bad or reject the report.
    ReportVote,
    /// Explain the decision with stable codes.
    Reasons,
}

impl RequestHeadRole {
    /// Whether a rule defining a head of this role decides something on its
    /// own. Reasons and a notification only explain or announce a decision.
    pub const fn decides(self) -> bool {
        matches!(
            self,
            Self::Vote | Self::Shaping | Self::Tags | Self::ReportVote
        )
    }

    /// Whether a head of this role decides a media request.
    pub const fn decides_request(self) -> bool {
        matches!(self, Self::Vote | Self::Shaping | Self::Tags)
    }
}

/// The shape of the value a head takes: the builder's value kinds, so the
/// request family serves the same schema shape as every other family.
pub use crate::rule_builder_schema::RuleBuilderValueKind as RequestValueKind;

/// The check each item of a list head passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestItemRule {
    Tag,
    AudioLanguage,
    ReasonCode,
}

impl RequestItemRule {
    pub fn check(self, item: &str) -> Result<(), String> {
        match self {
            Self::Tag => check_tag(item),
            Self::AudioLanguage => check_language(item),
            Self::ReasonCode => check_reason_code(item),
        }
    }
}

/// Everything the host knows about one head.
#[derive(Debug, Clone, Copy)]
pub struct RequestHeadSpec {
    /// Stable id, the GraphQL enum value of the head.
    pub id: &'static str,
    /// The Rego rule name an author writes.
    pub rule_name: &'static str,
    /// `None` for `reasons`, which the shared policy core decodes.
    pub action: Option<RequestActionHead>,
    pub role: RequestHeadRole,
    pub value_kind: RequestValueKind,
    /// The picker catalog a client offers values from.
    pub options: Option<&'static str>,
    /// Bounds of a number value.
    pub min: Option<i64>,
    pub max: Option<i64>,
    /// Bounds of a list value.
    pub max_items: Option<usize>,
    /// Bound of a text value or of each list item, in bytes.
    pub max_bytes: Option<usize>,
    /// The characters each list item may contain, as a regular expression.
    pub pattern: Option<&'static str>,
    pub item_rule: Option<RequestItemRule>,
    /// The Rego the builder writes; see [`RequestHeadSpec::fill_template`].
    pub template: &'static str,
    /// The template is written once per list item.
    pub per_item: bool,
    /// Choosing the head can lead to something being deleted.
    pub destructive: bool,
    /// Variants that may declare the head. `None` means every variant.
    pub variants: Option<&'static [&'static str]>,
    /// Ids of heads this one is never chosen with.
    pub conflicts_with: Option<&'static [&'static str]>,
    /// The Rego literal the wrapper uses when the head is undefined.
    pub wrapper_default: &'static str,
    /// A valid value, one entry per list item.
    pub sample: &'static [&'static str],
}

const VOTE_TEMPLATE: &str = "{head} if {helper}";
const VALUE_TEMPLATE: &str = "{head} := {value} if {helper}";
const LIST_TEMPLATE: &str = "{head} := [{items}] if {helper}";

const fn vote(
    id: &'static str,
    rule_name: &'static str,
    head: RequestActionHead,
) -> RequestHeadSpec {
    RequestHeadSpec {
        id,
        rule_name,
        action: Some(head),
        role: RequestHeadRole::Vote,
        value_kind: RequestValueKind::Flag,
        options: None,
        min: None,
        max: None,
        max_items: None,
        max_bytes: None,
        pattern: None,
        item_rule: None,
        template: VOTE_TEMPLATE,
        per_item: false,
        destructive: false,
        variants: None,
        conflicts_with: None,
        wrapper_default: "false",
        sample: &[],
    }
}

const fn valued(
    id: &'static str,
    rule_name: &'static str,
    head: RequestActionHead,
    role: RequestHeadRole,
    value_kind: RequestValueKind,
    sample: &'static [&'static str],
) -> RequestHeadSpec {
    RequestHeadSpec {
        role,
        value_kind,
        template: if matches!(value_kind, RequestValueKind::TextList) {
            LIST_TEMPLATE
        } else {
            VALUE_TEMPLATE
        },
        wrapper_default: "null",
        sample,
        ..vote(id, rule_name, head)
    }
}

const fn audio_languages(
    id: &'static str,
    rule_name: &'static str,
    head: RequestActionHead,
) -> RequestHeadSpec {
    RequestHeadSpec {
        options: Some("audioLanguages"),
        max_items: Some(MAX_AUDIO_LANGUAGES),
        max_bytes: Some(MAX_AUDIO_LANGUAGE_LEN),
        pattern: Some(AUDIO_LANGUAGE_PATTERN),
        item_rule: Some(RequestItemRule::AudioLanguage),
        ..valued(
            id,
            rule_name,
            head,
            RequestHeadRole::Shaping,
            RequestValueKind::TextList,
            &["eng"],
        )
    }
}

/// The heads that decide a media request. A report head is never chosen
/// with one of them.
const REQUEST_DECIDING_HEAD_IDS: &[&str] = &[
    "APPROVE",
    "DENY",
    "MANUAL",
    "SET_QUALITY_PROFILE",
    "SET_LEASE_DAYS",
    "SET_MONITOR",
    "SET_REQUIRED_AUDIO_LANGUAGES",
    "SET_PREFERRED_AUDIO_LANGUAGES",
    "ADD_TAGS",
];
/// The heads that decide a bad-file report.
const REPORT_HEAD_IDS: &[&str] = &["MARK_BAD", "REJECT_REPORT"];

const fn request_head(spec: RequestHeadSpec) -> RequestHeadSpec {
    RequestHeadSpec {
        conflicts_with: Some(REPORT_HEAD_IDS),
        ..spec
    }
}

const fn report_vote(
    id: &'static str,
    rule_name: &'static str,
    head: RequestActionHead,
) -> RequestHeadSpec {
    RequestHeadSpec {
        role: RequestHeadRole::ReportVote,
        conflicts_with: Some(REQUEST_DECIDING_HEAD_IDS),
        ..vote(id, rule_name, head)
    }
}

/// Every head a request rule may define, in the order the wrapper projects
/// them and the builder lists them. Entry `i` describes the head whose
/// discriminant is `i`; `reasons` is last.
pub const REQUEST_HEADS: [RequestHeadSpec; 13] = [
    request_head(vote("APPROVE", "approve", RequestActionHead::Approve)),
    request_head(vote("DENY", "deny", RequestActionHead::Deny)),
    request_head(vote("MANUAL", "manual", RequestActionHead::Manual)),
    RequestHeadSpec {
        options: Some("qualityProfiles"),
        conflicts_with: Some(REPORT_HEAD_IDS),
        ..valued(
            "SET_QUALITY_PROFILE",
            "set_quality_profile",
            RequestActionHead::SetQualityProfile,
            RequestHeadRole::Shaping,
            RequestValueKind::Choice,
            &["HD-1080p"],
        )
    },
    // A shorter lease than the requester asked for can make the title
    // eligible for lease-expiry deletion sooner.
    RequestHeadSpec {
        min: Some(0),
        max: Some(MAX_LEASE_DAYS),
        destructive: true,
        conflicts_with: Some(REPORT_HEAD_IDS),
        ..valued(
            "SET_LEASE_DAYS",
            "set_lease_days",
            RequestActionHead::SetLeaseDays,
            RequestHeadRole::Shaping,
            RequestValueKind::Number,
            &["30"],
        )
    },
    RequestHeadSpec {
        options: Some("monitorTypes"),
        conflicts_with: Some(REPORT_HEAD_IDS),
        ..valued(
            "SET_MONITOR",
            "set_monitor",
            RequestActionHead::SetMonitor,
            RequestHeadRole::Shaping,
            RequestValueKind::Choice,
            &["futureEpisodes"],
        )
    },
    request_head(audio_languages(
        "SET_REQUIRED_AUDIO_LANGUAGES",
        "set_required_audio_languages",
        RequestActionHead::SetRequiredAudioLanguages,
    )),
    request_head(audio_languages(
        "SET_PREFERRED_AUDIO_LANGUAGES",
        "set_preferred_audio_languages",
        RequestActionHead::SetPreferredAudioLanguages,
    )),
    RequestHeadSpec {
        options: Some("titleTags"),
        max_items: Some(MAX_TAGS),
        max_bytes: Some(MAX_TAG_LEN),
        pattern: Some(TAG_PATTERN),
        item_rule: Some(RequestItemRule::Tag),
        wrapper_default: "[]",
        conflicts_with: Some(REPORT_HEAD_IDS),
        ..valued(
            "ADD_TAGS",
            "add_tags",
            RequestActionHead::AddTags,
            RequestHeadRole::Tags,
            RequestValueKind::TextList,
            &["family"],
        )
    },
    RequestHeadSpec {
        max_bytes: Some(MAX_NOTIFY_LEN),
        ..valued(
            "NOTIFY",
            "notify",
            RequestActionHead::Notify,
            RequestHeadRole::Notify,
            RequestValueKind::Text,
            &["Approved by a rule"],
        )
    },
    report_vote("MARK_BAD", "mark_bad", RequestActionHead::MarkBad),
    report_vote(
        "REJECT_REPORT",
        "reject_report",
        RequestActionHead::RejectReport,
    ),
    RequestHeadSpec {
        id: "REASONS",
        rule_name: "reasons",
        action: None,
        role: RequestHeadRole::Reasons,
        value_kind: RequestValueKind::TextList,
        options: None,
        min: None,
        max: None,
        max_items: Some(MAX_REASON_CODES),
        max_bytes: Some(MAX_REASON_CODE_LEN),
        pattern: None,
        item_rule: Some(RequestItemRule::ReasonCode),
        template: "{head} contains {item} if {helper}",
        per_item: true,
        destructive: false,
        variants: None,
        conflicts_with: None,
        wrapper_default: "[]",
        sample: &["family_rated"],
    },
];

/// The table entry for a rule name, if the name is a head.
pub fn head_spec(rule_name: &str) -> Option<&'static RequestHeadSpec> {
    REQUEST_HEADS
        .iter()
        .find(|spec| spec.rule_name == rule_name)
}

impl RequestHeadSpec {
    /// Translation key of the head's name. The server sends no display text.
    pub fn label_key(&self) -> String {
        format!("ruleOutput.{REQUEST_RULE_FAMILY}.{}", self.rule_name)
    }

    /// The template written out for `items` and helper `helper`: `{head}` is
    /// the rule name, `{value}` the first item's literal, `{items}` every
    /// item's literal joined by commas and `{item}` one item's literal, the
    /// template repeated per item when [`RequestHeadSpec::per_item`] is set.
    pub fn fill_template(&self, helper: &str, items: &[&str]) -> String {
        let literal = |item: &str| match self.value_kind {
            RequestValueKind::Number => item.to_string(),
            _ => serde_json::Value::String(item.to_string()).to_string(),
        };
        let literals: Vec<String> = items.iter().map(|item| literal(item)).collect();
        let fill = |item: &str| {
            self.template
                .replace("{head}", self.rule_name)
                .replace("{helper}", helper)
                .replace("{value}", literals.first().map_or("", String::as_str))
                .replace("{items}", &literals.join(", "))
                .replace("{item}", item)
        };
        if self.per_item {
            literals
                .iter()
                .map(|item| fill(item))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            fill("")
        }
    }

    /// The template filled with the sample value, as an example in a message.
    fn example(&self) -> String {
        self.fill_template("{ ... }", self.sample)
    }
}

impl RequestActionHead {
    /// Every head, in the order the wrapper projects them: the entries of
    /// [`REQUEST_HEADS`] before `reasons`.
    pub const ALL: [Self; REQUEST_HEADS.len() - 1] = {
        let mut all = [Self::Approve; REQUEST_HEADS.len() - 1];
        let mut index = 0;
        while index < all.len() {
            all[index] = match REQUEST_HEADS[index].action {
                Some(head) => head,
                None => panic!("only the last request head may lack an action"),
            };
            index += 1;
        }
        all
    };

    pub const fn spec(self) -> &'static RequestHeadSpec {
        &REQUEST_HEADS[self as usize]
    }

    /// The Rego rule name an author writes.
    pub const fn rule_name(self) -> &'static str {
        self.spec().rule_name
    }

    pub fn from_rule_name(name: &str) -> Option<Self> {
        head_spec(name).and_then(|spec| spec.action)
    }

    /// Whether the head is one of the three votes.
    pub const fn is_vote(self) -> bool {
        matches!(self.spec().role, RequestHeadRole::Vote)
    }

    /// Whether the head changes what an approval writes.
    pub const fn is_shaping(self) -> bool {
        matches!(self.spec().role, RequestHeadRole::Shaping)
    }

    /// The heads that change what an approval writes, in table order.
    pub fn shaping() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|head| head.is_shaping())
    }
}

// ── Decoded values ──────────────────────────────────────────────────────────

/// One head's value, decoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestHeadValue {
    Number(i64),
    Text(String),
    List(Vec<String>),
}

/// The shaping values one rule, or the arbitration of several, set.
///
/// Each `None` leaves what the requester asked for in place.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestShaping {
    pub quality_profile: Option<String>,
    /// `0` keeps the title forever; otherwise 1 to [`MAX_LEASE_DAYS`].
    pub lease_days: Option<i64>,
    pub monitor: Option<String>,
    pub required_audio_languages: Option<Vec<String>>,
    pub preferred_audio_languages: Option<Vec<String>>,
}

impl RequestShaping {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The value one shaping head carries, or `None` when it carries none or
    /// the head is not a shaping head.
    pub fn get(&self, head: RequestActionHead) -> Option<RequestHeadValue> {
        match head {
            RequestActionHead::SetQualityProfile => {
                self.quality_profile.clone().map(RequestHeadValue::Text)
            }
            RequestActionHead::SetLeaseDays => self.lease_days.map(RequestHeadValue::Number),
            RequestActionHead::SetMonitor => self.monitor.clone().map(RequestHeadValue::Text),
            RequestActionHead::SetRequiredAudioLanguages => self
                .required_audio_languages
                .clone()
                .map(RequestHeadValue::List),
            RequestActionHead::SetPreferredAudioLanguages => self
                .preferred_audio_languages
                .clone()
                .map(RequestHeadValue::List),
            RequestActionHead::Approve
            | RequestActionHead::Deny
            | RequestActionHead::Manual
            | RequestActionHead::AddTags
            | RequestActionHead::Notify
            | RequestActionHead::MarkBad
            | RequestActionHead::RejectReport => None,
        }
    }

    /// Set or clear one shaping head. A value of the wrong shape, or a head
    /// that is not a shaping head, is ignored; the decode never produces one.
    pub fn set(&mut self, head: RequestActionHead, value: Option<RequestHeadValue>) {
        fn text(value: Option<RequestHeadValue>) -> Option<String> {
            match value {
                Some(RequestHeadValue::Text(text)) => Some(text),
                _ => None,
            }
        }
        fn list(value: Option<RequestHeadValue>) -> Option<Vec<String>> {
            match value {
                Some(RequestHeadValue::List(list)) => Some(list),
                _ => None,
            }
        }
        match head {
            RequestActionHead::SetQualityProfile => self.quality_profile = text(value),
            RequestActionHead::SetLeaseDays => {
                self.lease_days = match value {
                    Some(RequestHeadValue::Number(days)) => Some(days),
                    _ => None,
                }
            }
            RequestActionHead::SetMonitor => self.monitor = text(value),
            RequestActionHead::SetRequiredAudioLanguages => {
                self.required_audio_languages = list(value)
            }
            RequestActionHead::SetPreferredAudioLanguages => {
                self.preferred_audio_languages = list(value)
            }
            RequestActionHead::Approve
            | RequestActionHead::Deny
            | RequestActionHead::Manual
            | RequestActionHead::AddTags
            | RequestActionHead::Notify
            | RequestActionHead::MarkBad
            | RequestActionHead::RejectReport => {}
        }
    }

    /// The shaping heads that carry a value, in table order.
    pub fn heads(&self) -> Vec<RequestActionHead> {
        RequestActionHead::shaping()
            .filter(|head| self.get(*head).is_some())
            .collect()
    }
}

/// Everything but the vote and reasons one rule decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DecodedRequestActions {
    pub(crate) shaping: RequestShaping,
    pub(crate) add_tags: Vec<String>,
    pub(crate) notify: Option<String>,
}

/// Decode the non-vote heads of the wrapper's decision object.
///
/// Fails closed: a defined head of the wrong type or out of range is an error
/// for the rule, never a coerced or clamped value.
pub(crate) fn decode_request_actions(value: &Value) -> Result<DecodedRequestActions, String> {
    let mut decoded = DecodedRequestActions::default();
    for spec in &REQUEST_HEADS {
        let Some(head) = spec.action else {
            continue;
        };
        let head_value = match spec.role {
            RequestHeadRole::Vote | RequestHeadRole::ReportVote | RequestHeadRole::Reasons => {
                continue;
            }
            _ => decode_head(spec, &value[spec.rule_name])?,
        };
        match spec.role {
            RequestHeadRole::Shaping => decoded.shaping.set(head, head_value),
            RequestHeadRole::Tags => {
                if let Some(RequestHeadValue::List(tags)) = head_value {
                    decoded.add_tags = tags;
                }
            }
            RequestHeadRole::Notify => {
                if let Some(RequestHeadValue::Text(text)) = head_value {
                    decoded.notify = Some(text);
                }
            }
            RequestHeadRole::Vote | RequestHeadRole::ReportVote | RequestHeadRole::Reasons => {}
        }
    }
    Ok(decoded)
}

/// Decode one valued head as its table entry describes. `null`, the
/// wrapper's default for an undefined head, is `None`.
fn decode_head(spec: &RequestHeadSpec, value: &Value) -> Result<Option<RequestHeadValue>, String> {
    let name = spec.rule_name;
    if *value == Value::Null {
        return Ok(None);
    }
    match spec.value_kind {
        RequestValueKind::Flag => Ok(None),
        RequestValueKind::Text | RequestValueKind::Choice => {
            let text = value
                .as_string()
                .map_err(|_| format!("'{name}' must be a string"))?;
            check_text(spec, text)?;
            Ok(Some(RequestHeadValue::Text(text.to_string())))
        }
        RequestValueKind::Number => {
            let number = value
                .as_number()
                .ok()
                .and_then(|number| number.as_i64())
                .ok_or_else(|| format!("'{name}' must be a whole number"))?;
            check_number(spec, number)?;
            Ok(Some(RequestHeadValue::Number(number)))
        }
        RequestValueKind::TextList => {
            // A list, as the contract says: the static scan reads arrays only.
            let items = value
                .as_array()
                .map_err(|_| format!("'{name}' must be an array of strings"))?;
            check_list_len(spec, items.len())?;
            let mut list: Vec<String> = Vec::with_capacity(items.len());
            for item in items.iter() {
                let item = item
                    .as_string()
                    .map_err(|_| format!("'{name}' must contain only strings"))?;
                check_item(spec, item)?;
                if !list.iter().any(|seen| seen.as_str() == item.as_ref()) {
                    list.push(item.to_string());
                }
            }
            Ok(Some(RequestHeadValue::List(list)))
        }
    }
}

fn check_text(spec: &RequestHeadSpec, text: &str) -> Result<(), String> {
    let name = spec.rule_name;
    if text.is_empty() {
        return Err(format!("'{name}' must not be empty"));
    }
    if let Some(max) = spec.max_bytes
        && text.len() > max
    {
        return Err(format!("'{name}' is longer than {max} bytes"));
    }
    Ok(())
}

fn check_number(spec: &RequestHeadSpec, number: i64) -> Result<(), String> {
    let (min, max) = (spec.min.unwrap_or(i64::MIN), spec.max.unwrap_or(i64::MAX));
    if (min..=max).contains(&number) {
        return Ok(());
    }
    let name = spec.rule_name;
    Err(if spec.action == Some(RequestActionHead::SetLeaseDays) {
        format!("'{name}' must be 0 (forever) or 1 to {max} days, not {number}")
    } else {
        format!("'{name}' must be from {min} to {max}, not {number}")
    })
}

fn check_list_len(spec: &RequestHeadSpec, len: usize) -> Result<(), String> {
    match spec.max_items {
        Some(max) if len > max => Err(format!(
            "'{}' has {len} entries, at most {max} are allowed",
            spec.rule_name
        )),
        _ => Ok(()),
    }
}

fn check_item(spec: &RequestHeadSpec, item: &str) -> Result<(), String> {
    match spec.item_rule {
        Some(rule) => rule.check(item),
        None => Ok(()),
    }
}

/// Whether one audio language code has a shape a rule may write. Whether
/// Scryer knows the language is the host's question.
fn check_language(language: &str) -> Result<(), String> {
    if language.is_empty() {
        return Err("an audio language must not be empty".to_string());
    }
    if language.len() > MAX_AUDIO_LANGUAGE_LEN {
        return Err(format!(
            "audio language is longer than {MAX_AUDIO_LANGUAGE_LEN} bytes"
        ));
    }
    if let Some(bad) = language
        .chars()
        .find(|c| !(c.is_ascii_alphabetic() || matches!(c, '_' | '-')))
    {
        return Err(format!(
            "audio language '{language}' contains unsupported character '{bad}'; use letters, \
             '_' or '-'"
        ));
    }
    Ok(())
}

/// A literal reason code is at most [`MAX_REASON_CODE_LEN`] bytes of UTF-8.
fn check_reason_code(reason: &str) -> Result<(), String> {
    if reason.len() > MAX_REASON_CODE_LEN {
        return Err(format!(
            "reason code is {} bytes, at most {MAX_REASON_CODE_LEN} are allowed",
            reason.len()
        ));
    }
    Ok(())
}

// ── Builder schema ──────────────────────────────────────────────────────────

/// The request rule builder schema, read off [`REQUEST_HEADS`].
pub fn request_builder_schema() -> RuleBuilderSchema {
    let ids = |keep: fn(&RequestHeadSpec) -> bool| {
        REQUEST_HEADS
            .iter()
            .filter(|spec| keep(spec))
            .map(|spec| spec.id)
            .collect()
    };
    RuleBuilderSchema {
        family: REQUEST_RULE_FAMILY,
        subjects: Vec::new(),
        outputs: REQUEST_HEADS
            .iter()
            .map(|spec| RuleBuilderOutputDescriptor {
                id: spec.id,
                head: spec.rule_name,
                label_key: spec.label_key(),
                action_label_key: None,
                help_key: None,
                value_kind: spec.value_kind,
                options: spec.options,
                min: spec.min,
                max: spec.max,
                max_items: spec.max_items,
                max_bytes: spec.max_bytes,
                pattern: spec.pattern,
                template: spec.template,
                per_item: spec.per_item,
                destructive: spec.destructive,
                variants: spec.variants,
                conflicts_with: spec.conflicts_with.unwrap_or_default().to_vec(),
            })
            .collect(),
        output_rules: vec![
            // One rule casts one vote: manual beats deny beats approve, so a
            // second would never count.
            RuleBuilderOutputRule {
                rule: RuleBuilderOutputRuleKind::AtMostOne,
                of: Some(ids(|spec| spec.role == RequestHeadRole::Vote)),
            },
            // A report rule marks the file bad or rejects the report, not
            // both.
            RuleBuilderOutputRule {
                rule: RuleBuilderOutputRuleKind::AtMostOne,
                of: Some(ids(|spec| spec.role == RequestHeadRole::ReportVote)),
            },
            // Reasons and a notification alone decide nothing, so the server
            // refuses a rule that writes only those.
            RuleBuilderOutputRule {
                rule: RuleBuilderOutputRuleKind::AtLeastOne,
                of: Some(ids(|spec| spec.role.decides())),
            },
        ],
    }
}

// ── Static head scan ────────────────────────────────────────────────────────

/// A whole-number literal written for a number head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleNumberLiteral {
    pub value: i64,
    /// One-based line and character column of the literal.
    pub line: usize,
    pub column: usize,
}

/// What a request rule's source can decide, read without running it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RequestActionScan {
    /// Every head the source defines, through any rule or `default`.
    pub defined_heads: BTreeSet<RequestActionHead>,
    /// Whether the source defines `reasons`.
    pub defines_reasons: bool,
    /// Every text literal written for each valued head, across all
    /// definitions and `else` branches, in source order.
    pub text_literals: BTreeMap<RequestActionHead, Vec<RuleLiteral>>,
    /// Every number literal written for each number head, in source order.
    pub number_literals: BTreeMap<RequestActionHead, Vec<RuleNumberLiteral>>,
    /// Conditions that refuse the rule.
    pub errors: Vec<RuleScanIssue>,
    /// Conditions worth telling the author about that do not refuse the rule.
    pub warnings: Vec<RuleScanIssue>,
}

impl RequestActionScan {
    /// Whether the source decides anything: a vote, a shaping head, or a tag.
    /// `reasons` and `notify` alone explain or announce a decision nobody
    /// made.
    pub fn decides_something(&self) -> bool {
        self.defined_heads
            .iter()
            .any(|head| head.spec().role.decides())
    }

    /// Whether the source decides a media request.
    pub fn decides_request(&self) -> bool {
        self.defined_heads
            .iter()
            .any(|head| head.spec().role.decides_request())
    }

    /// Whether the source decides a bad-file report.
    pub fn decides_report(&self) -> bool {
        self.defined_heads
            .iter()
            .any(|head| head.spec().role == RequestHeadRole::ReportVote)
    }

    /// The text literals written for one head, in source order.
    pub fn literals(&self, head: RequestActionHead) -> &[RuleLiteral] {
        self.text_literals.get(&head).map_or(&[], Vec::as_slice)
    }

    /// The number literals written for one head, in source order.
    pub fn numbers(&self, head: RequestActionHead) -> &[RuleNumberLiteral] {
        self.number_literals.get(&head).map_or(&[], Vec::as_slice)
    }

    /// The declared actions summary: the heads in table order and each
    /// valued head's distinct literals in source order.
    pub fn declared_actions(&self) -> RequestDeclaredActions {
        let mut declared = RequestDeclaredActions {
            heads: RequestActionHead::ALL
                .into_iter()
                .filter(|head| self.defined_heads.contains(head))
                .collect(),
            ..RequestDeclaredActions::default()
        };
        for head in RequestActionHead::ALL {
            match declared.slot_mut(head) {
                Some(DeclaredSlot::Texts(values)) => {
                    for literal in self.literals(head) {
                        if !values.contains(&literal.value) {
                            values.push(literal.value.clone());
                        }
                    }
                }
                Some(DeclaredSlot::Numbers(values)) => {
                    for literal in self.numbers(head) {
                        if !values.contains(&literal.value) {
                            values.push(literal.value);
                        }
                    }
                }
                None => {}
            }
        }
        declared
    }
}

/// What a request rule's source can do, as the API reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RequestDeclaredActions {
    pub heads: Vec<RequestActionHead>,
    pub quality_profiles: Vec<String>,
    pub lease_days: Vec<i64>,
    pub monitors: Vec<String>,
    pub required_audio_languages: Vec<String>,
    pub preferred_audio_languages: Vec<String>,
    pub add_tags: Vec<String>,
    pub notifications: Vec<String>,
}

/// Where one head's declared values go.
pub enum DeclaredSlot<'a> {
    Texts(&'a mut Vec<String>),
    Numbers(&'a mut Vec<i64>),
}

impl RequestDeclaredActions {
    /// The field holding one head's declared values; `None` for a vote.
    pub fn slot_mut(&mut self, head: RequestActionHead) -> Option<DeclaredSlot<'_>> {
        Some(match head {
            RequestActionHead::Approve
            | RequestActionHead::Deny
            | RequestActionHead::Manual
            | RequestActionHead::MarkBad
            | RequestActionHead::RejectReport => {
                return None;
            }
            RequestActionHead::SetQualityProfile => DeclaredSlot::Texts(&mut self.quality_profiles),
            RequestActionHead::SetLeaseDays => DeclaredSlot::Numbers(&mut self.lease_days),
            RequestActionHead::SetMonitor => DeclaredSlot::Texts(&mut self.monitors),
            RequestActionHead::SetRequiredAudioLanguages => {
                DeclaredSlot::Texts(&mut self.required_audio_languages)
            }
            RequestActionHead::SetPreferredAudioLanguages => {
                DeclaredSlot::Texts(&mut self.preferred_audio_languages)
            }
            RequestActionHead::AddTags => DeclaredSlot::Texts(&mut self.add_tags),
            RequestActionHead::Notify => DeclaredSlot::Texts(&mut self.notifications),
        })
    }
}

/// Scan a stored request rule's source for what it can decide.
///
/// Fails only when the source does not parse; every other condition is an
/// issue on the result, and [`crate::validation::validate_request_rule`]
/// reports the same ones.
pub fn scan_request_action_heads(
    rego_source: &str,
    rule_set_id: &str,
) -> Result<RequestActionScan, RulesError> {
    let module = parse_module(
        rego_source,
        &crate::request::user_policy_path(rule_set_id),
        &RuntimeLimits::request_defaults(),
    )
    .map_err(RulesError::Compilation)?;
    Ok(scan_request_module(&module))
}

fn not_literal(spec: &RequestHeadSpec, at: (usize, usize)) -> RuleScanIssue {
    let name = spec.rule_name;
    let expected = match spec.value_kind {
        RequestValueKind::Number => "a whole number literal",
        RequestValueKind::TextList => "an array of string literals",
        RequestValueKind::Text | RequestValueKind::Choice | RequestValueKind::Flag => {
            "a string literal"
        }
    };
    issue(
        diagnostic_codes::RULE_ACTION_VALUE_NOT_LITERAL,
        name,
        format!(
            "'{name}' must be assigned {expected}, for example: {}",
            spec.example()
        ),
        at,
    )
}

fn invalid(spec: &RequestHeadSpec, message: String, at: (usize, usize)) -> RuleScanIssue {
    issue(
        diagnostic_codes::RULE_ACTION_VALUE_INVALID,
        spec.rule_name,
        message,
        at,
    )
}

pub(crate) fn scan_request_module(module: &Module) -> RequestActionScan {
    let mut scan = RequestActionScan::default();
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
        let is_function = is_function_rule(rule);

        let Some(spec) = head_spec(name) else {
            continue;
        };
        let Some(head) = spec.action else {
            // `reasons`, which the shared policy core reads.
            if is_function {
                scan.errors.push(head_defined_as_function(name, at));
            } else {
                scan.defines_reasons = true;
            }
            continue;
        };
        if is_function {
            scan.errors
                .push(if spec.value_kind == RequestValueKind::Flag {
                    head_defined_as_function(name, at)
                } else {
                    not_literal(spec, at)
                });
            continue;
        }
        scan.defined_heads.insert(head);
        if spec.value_kind != RequestValueKind::Flag {
            scan_value_rule(&mut scan, head, rule, refr);
        }
    }

    scan.errors.extend(foreign_data_references(module));

    let referenced = referenced_names(module);
    for (name, at) in defined {
        if head_spec(&name).is_some() || referenced.contains(&name) {
            continue;
        }
        scan.warnings.push(issue(
            diagnostic_codes::RULE_UNREFERENCED_RULE,
            &name,
            format!(
                "'{name}' is not a request rule head and nothing in this rule uses it; check \
                 the spelling if it was meant to be one"
            ),
            at,
        ));
    }

    scan
}

/// Read one definition of a valued head: record its literals, or refuse it.
fn scan_value_rule(
    scan: &mut RequestActionScan,
    head: RequestActionHead,
    rule: &regorus::unstable::Rule,
    refr: &Expr,
) {
    let spec = head.spec();
    let values = match plain_value_exprs(rule, refr) {
        Ok(values) => values,
        Err(at) => {
            scan.errors.push(not_literal(spec, at));
            return;
        }
    };

    for value in values {
        let Some(value) = value else {
            scan.errors.push(not_literal(spec, source_position(refr)));
            continue;
        };
        match spec.value_kind {
            RequestValueKind::Number => scan_number_value(scan, head, value),
            RequestValueKind::Text | RequestValueKind::Choice => {
                scan_text_value(scan, head, value);
            }
            RequestValueKind::TextList => scan_list_value(scan, head, value),
            RequestValueKind::Flag => {}
        }
    }
}

fn scan_number_value(scan: &mut RequestActionScan, head: RequestActionHead, value: &Expr) {
    let spec = head.spec();
    let at = source_position(value);
    let Expr::Number { value: number, .. } = value else {
        scan.errors.push(not_literal(spec, at));
        return;
    };
    let Some(number) = number.as_number().ok().and_then(|number| number.as_i64()) else {
        scan.errors.push(invalid(
            spec,
            format!("'{}' must be a whole number", spec.rule_name),
            at,
        ));
        return;
    };
    match check_number(spec, number) {
        Ok(()) => scan
            .number_literals
            .entry(head)
            .or_default()
            .push(RuleNumberLiteral {
                value: number,
                line: at.0,
                column: at.1,
            }),
        Err(message) => scan.errors.push(invalid(spec, message, at)),
    }
}

fn scan_text_value(scan: &mut RequestActionScan, head: RequestActionHead, value: &Expr) {
    let spec = head.spec();
    let at = source_position(value);
    let Some(text) = string_literal(value) else {
        scan.errors.push(not_literal(spec, at));
        return;
    };
    match check_text(spec, &text) {
        Ok(()) => scan
            .text_literals
            .entry(head)
            .or_default()
            .push(RuleLiteral {
                value: text,
                line: at.0,
                column: at.1,
            }),
        Err(message) => scan.errors.push(invalid(spec, message, at)),
    }
}

fn scan_list_value(scan: &mut RequestActionScan, head: RequestActionHead, value: &Expr) {
    let spec = head.spec();
    let Expr::Array { items, .. } = value else {
        scan.errors.push(not_literal(spec, source_position(value)));
        return;
    };
    if items.iter().any(|item| string_literal(item).is_none()) {
        scan.errors.push(not_literal(spec, source_position(value)));
        return;
    }
    if let Err(message) = check_list_len(spec, items.len()) {
        scan.errors
            .push(invalid(spec, message, source_position(value)));
        return;
    }
    for item in items {
        let Some(text) = string_literal(item) else {
            continue;
        };
        let at = source_position(item);
        match check_item(spec, &text) {
            Ok(()) => scan
                .text_literals
                .entry(head)
                .or_default()
                .push(RuleLiteral {
                    value: text,
                    line: at.0,
                    column: at.1,
                }),
            Err(message) => scan.errors.push(invalid(spec, message, at)),
        }
    }
}
