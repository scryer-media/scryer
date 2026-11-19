//! Translates Maintainerr rule groups into Scryer lifecycle rules.
//!
//! The translation is a pure function of two documents an operator's own
//! Maintainerr instance serves, `GET /api/rules` (the rule groups) and
//! `GET /api/rules/constants` (the property catalogue), plus what the caller
//! knows about Scryer and the Sonarr and Radarr instances the groups name. It
//! performs no I/O, reads no clock and stores nothing.
//!
//! Each rule group becomes one lifecycle rule, in input order. A group
//! translates only when every rule in it, and its action, has a faithful
//! Scryer equivalent. Anything else yields a FAILED rule: the web editor's
//! starter source with a description saying what could not be translated,
//! so the operator sees the group and decides what to do with it.
//!
//! Properties bind by application and property *name* through the instance's
//! own catalogue, never by Maintainerr's numeric ids, which differ between
//! releases. Rules fold with Maintainerr's own section semantics: within a
//! section each rule is OR'ed or AND'ed onto the running result in order, and
//! each section is then OR'ed or AND'ed onto the group's result in order.
//!
//! These parts of a Maintainerr group are ignored without a diagnostic,
//! because they configure Maintainerr itself rather than which media match:
//!
//! - the collection the group maintains on the media server, and whether
//!   media is shown in it;
//! - manual exclusions and the "list exclusions" setting of the collection;
//! - notification settings.
//!
//! The group's library is carried through as `source_library_id` for the
//! caller to map; the translated rule itself is not scoped to a library.
//! The group's own `isActive` is carried through as `source_is_active`. A
//! rule row's `isActive` is ignored, as Maintainerr's evaluator ignores it,
//! and rules are evaluated in the order of their row ids, as Maintainerr
//! loads them.

mod bind;
mod decode;
mod rego;

#[cfg(test)]
mod tests;

use crate::error_reasons::maintainerr_import as reasons;
use crate::{AppError, RefusalFact};
use std::collections::{HashMap, HashSet};

/// The largest export document accepted, in UTF-8 bytes, for each input.
pub const MAX_MAINTAINERR_JSON_BYTES: usize = 16 * 1024 * 1024;
/// The most rule groups one import accepts.
pub const MAX_MAINTAINERR_RULE_GROUPS: usize = 1000;
/// The most rules one group may hold and still translate.
pub const MAX_MAINTAINERR_RULES_PER_GROUP: usize = 100;
/// The per-run maximum given to a translated rule that deletes.
pub const MAINTAINERR_DESTRUCTIVE_MAX_TITLES_PER_RUN: u32 = 10;
/// What the name of a translated rule starts with, before the group's name.
pub const MAINTAINERR_NAME_PREFIX: &str = "(Maintainerr) ";
/// What the name of an untranslated rule starts with, before the group's name.
pub const MAINTAINERR_FAILED_NAME_PREFIX: &str = "(Maintainerr) FAILED - ";

/// The web editor's source for a new rule, byte for byte. A group that cannot
/// be translated is imported with this source.
pub const LIFECYCLE_STARTER_SOURCE: &str =
    "# Lifecycle rule. Defining a rule named after an action declares that
# action: unmonitor, monitor, search, fetch_subtitles, delete_files and
# delete_title take `if { ... }`; set_quality_profile, move_to_library,
# move_to_root and notify take a string, add_tags and remove_tags a list.
# On a series, monitor_season, unmonitor_season, delete_season_files,
# fetch_season_subtitles and notify_season act on the season at
# `input.season`; the _episode heads act on the episode at `input.episode`.
# Any other rule name is your own helper. `reasons` explains the choice.
#
# Facts are plain values: `input.facts.monitored` is the boolean itself. A
# fact with nothing to report is simply missing, so `not input.facts.has_file`
# is how you ask for \"no files\". If a fact Scryer could not observe is one this
# rule reads, Scryer holds the subject for you — you never write that guard.
# `days_since(ts)` turns a timestamp fact into whole days.

unmonitored_with_files if {
\tnot input.facts.monitored
\tinput.facts.has_file
}

add_tags := [\"needs-review\"] if unmonitored_with_files

reasons contains \"unmonitored_with_files\" if unmonitored_with_files
";

/// The source a group that cannot be translated is imported with: the web
/// editor's starter, except that it notifies instead of tagging. A tag must be
/// registered before a rule may write it, and the import never creates one,
/// so the starter's tag would refuse every untranslated group.
pub const MAINTAINERR_FAILED_SOURCE: &str =
    "# Lifecycle rule. Defining a rule named after an action declares that
# action: unmonitor, monitor, search, fetch_subtitles, delete_files and
# delete_title take `if { ... }`; set_quality_profile, move_to_library,
# move_to_root and notify take a string, add_tags and remove_tags a list.
# On a series, monitor_season, unmonitor_season, delete_season_files,
# fetch_season_subtitles and notify_season act on the season at
# `input.season`; the _episode heads act on the episode at `input.episode`.
# Any other rule name is your own helper. `reasons` explains the choice.
#
# Facts are plain values: `input.facts.monitored` is the boolean itself. A
# fact with nothing to report is simply missing, so `not input.facts.has_file`
# is how you ask for \"no files\". If a fact Scryer could not observe is one this
# rule reads, Scryer holds the subject for you — you never write that guard.
# `days_since(ts)` turns a timestamp fact into whole days.

unmonitored_with_files if {
\tnot input.facts.monitored
\tinput.facts.has_file
}

notify := \"Unmonitored with files.\" if unmonitored_with_files

reasons contains \"unmonitored_with_files\" if unmonitored_with_files
";

/// Stable diagnostic codes. Clients branch on these, never on the message.
pub mod codes {
    /// The group's rules, as Maintainerr folds them, can never match.
    pub const GROUP_INVALID: &str = "maintainerr.group.invalid";
    /// The group does not use rules (`useRules` is false).
    pub const GROUP_RULES_DISABLED: &str = "maintainerr.group.rules_disabled";
    /// The group holds no rules.
    pub const GROUP_RULES_EMPTY: &str = "maintainerr.group.rules_empty";
    /// The group holds more rules than one translated rule may.
    pub const GROUP_RULES_TOO_MANY: &str = "maintainerr.group.rules_too_many";
    /// The group's media type is missing or not one Scryer can act on.
    pub const GROUP_MEDIA_TYPE_UNSUPPORTED: &str = "maintainerr.group.media_type_unsupported";
    /// A rule is malformed.
    pub const RULE_INVALID: &str = "maintainerr.rule.invalid";
    /// A rule compares against another property that is missing.
    pub const RULE_SECOND_VALUE_MISSING: &str = "maintainerr.rule.second_value_missing";
    /// A per-user property has no username.
    pub const RULE_USERNAME_MISSING: &str = "maintainerr.rule.username_missing";
    /// A rule names an application the instance's catalogue does not list,
    /// usually one that is not configured on that Maintainerr instance.
    pub const APPLICATION_UNKNOWN: &str = "maintainerr.application.unknown";
    /// A rule names a property the instance's catalogue does not list.
    pub const PROPERTY_UNKNOWN: &str = "maintainerr.property.unknown";
    /// A property has no faithful Scryer fact.
    pub const PROPERTY_UNMAPPED: &str = "maintainerr.property.unmapped";
    /// A property reads media-server collections, playlists or labels.
    pub const PROPERTY_COLLECTION_UNSUPPORTED: &str = "maintainerr.property.collection_unsupported";
    /// A property has a Scryer fact, but not for this group's media type.
    pub const PROPERTY_LEVEL_UNSUPPORTED: &str = "maintainerr.property.level_unsupported";
    /// A comparison cannot be expressed faithfully for the property.
    pub const COMPARATOR_UNSUPPORTED: &str = "maintainerr.comparator.unsupported";
    /// Date equality depends on Maintainerr's local time zone.
    pub const COMPARATOR_DATE_EQUALITY_UNSUPPORTED: &str =
        "maintainerr.comparator.date_equality_unsupported";
    /// A rule's custom value cannot be read as Maintainerr reads it.
    pub const CUSTOM_VALUE_INVALID: &str = "maintainerr.custom_value.invalid";
    /// The group has no action.
    pub const ACTION_MISSING: &str = "maintainerr.action.missing";
    /// The group's action has no Scryer equivalent for its media type.
    pub const ACTION_UNSUPPORTED: &str = "maintainerr.action.unsupported";
    /// The group's quality profile is not one the Sonarr or Radarr instance lists.
    pub const QUALITY_PROFILE_UNRESOLVED: &str = "maintainerr.quality_profile.unresolved";
    /// No Scryer quality profile has the group's quality profile name.
    pub const QUALITY_PROFILE_NOT_IN_SCRYER: &str = "maintainerr.quality_profile.not_in_scryer";
    /// The generated source exceeds the lifecycle rule size limits.
    pub const SOURCE_TOO_LARGE: &str = "maintainerr.source.too_large";
    /// The generated source did not pass lifecycle rule validation.
    pub const SOURCE_INVALID: &str = "maintainerr.source.invalid";
    /// A rule names a user no Scryer user is called. It still translates.
    pub const USERNAME_UNKNOWN: &str = "maintainerr.username.unknown";
    /// A property maps to a Scryer fact that differs from it in a stated way.
    pub const FACT_APPROXIMATE: &str = "maintainerr.fact.approximate";
    /// The action maps to Scryer heads that differ from it in a stated way.
    pub const ACTION_APPROXIMATE: &str = "maintainerr.action.approximate";

    /// Every diagnostic code the translator emits.
    pub const ALL: &[&str] = &[
        GROUP_INVALID,
        GROUP_RULES_DISABLED,
        GROUP_RULES_EMPTY,
        GROUP_RULES_TOO_MANY,
        GROUP_MEDIA_TYPE_UNSUPPORTED,
        RULE_INVALID,
        RULE_SECOND_VALUE_MISSING,
        RULE_USERNAME_MISSING,
        APPLICATION_UNKNOWN,
        PROPERTY_UNKNOWN,
        PROPERTY_UNMAPPED,
        PROPERTY_COLLECTION_UNSUPPORTED,
        PROPERTY_LEVEL_UNSUPPORTED,
        COMPARATOR_UNSUPPORTED,
        COMPARATOR_DATE_EQUALITY_UNSUPPORTED,
        CUSTOM_VALUE_INVALID,
        ACTION_MISSING,
        ACTION_UNSUPPORTED,
        QUALITY_PROFILE_UNRESOLVED,
        QUALITY_PROFILE_NOT_IN_SCRYER,
        SOURCE_TOO_LARGE,
        SOURCE_INVALID,
        USERNAME_UNKNOWN,
        FACT_APPROXIMATE,
        ACTION_APPROXIMATE,
    ];
}

/// Which *arr application a quality profile id belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrKind {
    Radarr,
    Sonarr,
}

/// What the caller knows about Scryer and the *arr instances a group names.
#[derive(Debug, Clone, Default)]
pub struct MaintainerrTranslateContext {
    /// Quality profile names by (application, Maintainerr settings id, profile id).
    pub arr_profile_names: HashMap<(ArrKind, i64, i64), String>,
    /// The names of Scryer's quality profiles.
    pub scryer_quality_profile_names: HashSet<String>,
    /// The usernames of Scryer's users.
    pub scryer_usernames: HashSet<String>,
}

impl MaintainerrTranslateContext {
    /// Records the quality profiles one Sonarr or Radarr instance lists, from
    /// the body of Maintainerr's `GET /api/servarr/{radarr|sonarr}/:id/profiles`
    /// for the settings id `settings_id`: an array of profiles, each with an
    /// `id` and a `name`.
    pub fn add_arr_profiles(
        &mut self,
        kind: ArrKind,
        settings_id: i64,
        profiles_json: &str,
    ) -> Result<(), MaintainerrImportRefusal> {
        if profiles_json.len() > MAX_MAINTAINERR_JSON_BYTES {
            return Err(MaintainerrImportRefusal::JsonTooLarge);
        }
        let profiles: serde_json::Value = serde_json::from_str(profiles_json)
            .map_err(|_| MaintainerrImportRefusal::ProfilesJsonInvalid)?;
        self.add_arr_profile_values(kind, settings_id, &profiles)
    }

    /// [`Self::add_arr_profiles`] for a body already parsed.
    pub fn add_arr_profile_values(
        &mut self,
        kind: ArrKind,
        settings_id: i64,
        profiles: &serde_json::Value,
    ) -> Result<(), MaintainerrImportRefusal> {
        let invalid = MaintainerrImportRefusal::ProfilesJsonInvalid;
        let profiles = profiles.as_array().ok_or(invalid)?;
        let mut found = Vec::with_capacity(profiles.len());
        for profile in profiles {
            let id = profile
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .ok_or(invalid)?;
            let name = profile
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or(invalid)?;
            found.push(((kind, settings_id, id), name.to_string()));
        }
        self.arr_profile_names.extend(found);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintainerrRuleStatus {
    Translated,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintainerrDiagnosticSeverity {
    Error,
    Warning,
}

/// One finding about a rule group. `code` is one of [`codes::ALL`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintainerrDiagnostic {
    pub code: &'static str,
    pub severity: MaintainerrDiagnosticSeverity,
    pub message: String,
    /// The zero-based position of the rule in the group, when one is at fault.
    pub rule_index: Option<usize>,
}

/// One rule group, translated or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintainerrTranslatedRule {
    pub source_group_id: i64,
    pub source_group_name: String,
    /// Maintainerr's library id for the group, for the caller to map.
    pub source_library_id: Option<String>,
    /// Whether the group was active in Maintainerr.
    pub source_is_active: bool,
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub grace_days: i64,
    pub max_titles_per_run: Option<u32>,
    pub status: MaintainerrRuleStatus,
    /// Whether the source reads a fact about identifiable people, which needs
    /// permission-management authority to save.
    pub reads_person_targeted_fact: bool,
    pub diagnostics: Vec<MaintainerrDiagnostic>,
}

/// An export the translator cannot read at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintainerrImportRefusal {
    JsonTooLarge,
    GroupsJsonInvalid,
    ConstantsJsonInvalid,
    GroupsTooMany,
    ProfilesJsonInvalid,
}

impl MaintainerrImportRefusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::JsonTooLarge => reasons::JSON_TOO_LARGE,
            Self::GroupsJsonInvalid => reasons::GROUPS_JSON_INVALID,
            Self::ConstantsJsonInvalid => reasons::CONSTANTS_JSON_INVALID,
            Self::GroupsTooMany => reasons::GROUPS_TOO_MANY,
            Self::ProfilesJsonInvalid => reasons::PROFILES_JSON_INVALID,
        }
    }

    pub fn message(self) -> String {
        match self {
            Self::JsonTooLarge => {
                format!("Maintainerr sent more than {MAX_MAINTAINERR_JSON_BYTES} bytes of JSON.")
            }
            Self::GroupsJsonInvalid => {
                "Maintainerr's rule groups are not in the expected form.".to_string()
            }
            Self::ConstantsJsonInvalid => {
                "Maintainerr's rule properties are not in the expected form.".to_string()
            }
            Self::GroupsTooMany => format!(
                "Maintainerr has more than {MAX_MAINTAINERR_RULE_GROUPS} rule groups to import."
            ),
            Self::ProfilesJsonInvalid => {
                "A Sonarr or Radarr quality profile list is not in the expected form.".to_string()
            }
        }
    }

    fn facts(self) -> Vec<RefusalFact> {
        match self {
            Self::JsonTooLarge => vec![RefusalFact::new(
                reasons::MAX_BYTES_FACT,
                MAX_MAINTAINERR_JSON_BYTES.to_string(),
            )],
            Self::GroupsTooMany => vec![RefusalFact::new(
                reasons::MAX_GROUPS_FACT,
                MAX_MAINTAINERR_RULE_GROUPS.to_string(),
            )],
            Self::GroupsJsonInvalid | Self::ConstantsJsonInvalid | Self::ProfilesJsonInvalid => {
                Vec::new()
            }
        }
    }

    pub fn into_app_error(self) -> AppError {
        AppError::validation_refused_with_facts(self.reason(), self.message(), self.facts())
    }
}

/// Translates every rule group in `groups_json` (the body of Maintainerr's
/// `GET /api/rules`), binding properties through `constants_json` (the body
/// of `GET /api/rules/constants`). The result has one entry per group, in
/// input order.
pub fn translate_maintainerr_rule_groups(
    groups_json: &str,
    constants_json: &str,
    ctx: &MaintainerrTranslateContext,
) -> Result<Vec<MaintainerrTranslatedRule>, MaintainerrImportRefusal> {
    if groups_json.len() > MAX_MAINTAINERR_JSON_BYTES
        || constants_json.len() > MAX_MAINTAINERR_JSON_BYTES
    {
        return Err(MaintainerrImportRefusal::JsonTooLarge);
    }
    let catalogue = decode::decode_catalogue(constants_json)?;
    let groups = decode::decode_groups(groups_json)?;
    Ok(translate_decoded(&groups, &catalogue, ctx))
}

/// [`translate_maintainerr_rule_groups`] for bodies already parsed: `groups`
/// is the array `GET /api/rules` returns and `constants` the body of
/// `GET /api/rules/constants`.
pub fn translate_maintainerr_rule_group_values(
    groups: &[serde_json::Value],
    constants: &serde_json::Value,
    ctx: &MaintainerrTranslateContext,
) -> Result<Vec<MaintainerrTranslatedRule>, MaintainerrImportRefusal> {
    let catalogue = decode::decode_catalogue_value(constants)?;
    let groups = decode::decode_group_values(groups)?;
    Ok(translate_decoded(&groups, &catalogue, ctx))
}

fn translate_decoded(
    groups: &[decode::DecodedGroup],
    catalogue: &decode::Catalogue,
    ctx: &MaintainerrTranslateContext,
) -> Vec<MaintainerrTranslatedRule> {
    let known_usernames: HashSet<String> = ctx
        .scryer_usernames
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    groups
        .iter()
        .map(|group| rego::translate_group(group, catalogue, ctx, &known_usernames))
        .collect()
}
