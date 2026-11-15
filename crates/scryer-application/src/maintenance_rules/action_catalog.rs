//! Closed vocabularies shared by the maintenance action sequence and history.
//!
//! Rules declare their actions in their own Rego source. What remains here is
//! the subject, risk and effect vocabulary the step catalog is written in, and
//! the names of the single actions earlier builds recorded on action runs,
//! which history still has to label.

use serde::{Deserialize, Serialize};

/// The single actions earlier builds recorded on action runs and candidates.
///
/// Nothing executes these any more. A stored run row still carries one of
/// these names, so history reads it back through this closed set; an
/// unrecognized name is never mapped onto a fallback.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceActionKind {
    /// Membership/collection tracking only; no media mutation.
    DoNothing,
    /// Unmonitor the matched scope and keep its files.
    UnmonitorScopeKeepFiles,
    /// Delete the title and its files.
    DeleteTitleAndFiles,
    /// Unmonitor the title and delete its files while preserving the title.
    UnmonitorTitleDeleteAllFiles,
    /// Unmonitor the show and seasons and delete existing episode files.
    UnmonitorShowDeleteExistingFiles,
    /// Unmonitor the season or episode scope and delete its files.
    UnmonitorScopeDeleteFiles,
    /// Unmonitor the season and delete its files, then delete the show when a
    /// fresh parent check proves it is empty.
    UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
    /// Unmonitor the season, then unmonitor the show when a fresh parent check
    /// proves it is empty.
    UnmonitorSeasonThenUnmonitorShowIfEmpty,
    /// Change the quality profile and search only when it changed.
    ChangeQualityProfileAndSearchIfChanged,
    /// Add user tags to the matched title.
    AddTags,
    /// Remove user tags from the matched title.
    RemoveTags,
}

impl MaintenanceActionKind {
    /// Every recorded kind.
    pub const ALL: &'static [Self] = &[
        Self::DoNothing,
        Self::UnmonitorScopeKeepFiles,
        Self::DeleteTitleAndFiles,
        Self::UnmonitorTitleDeleteAllFiles,
        Self::UnmonitorShowDeleteExistingFiles,
        Self::UnmonitorScopeDeleteFiles,
        Self::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
        Self::UnmonitorSeasonThenUnmonitorShowIfEmpty,
        Self::ChangeQualityProfileAndSearchIfChanged,
        Self::AddTags,
        Self::RemoveTags,
    ];

    /// The pinned wire name, identical to what serde serializes. A run row
    /// stores the action by this name rather than by a second encoding, so
    /// `action_kind_wire_names_are_pinned` covers both at once.
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::DoNothing => "do_nothing",
            Self::UnmonitorScopeKeepFiles => "unmonitor_scope_keep_files",
            Self::DeleteTitleAndFiles => "delete_title_and_files",
            Self::UnmonitorTitleDeleteAllFiles => "unmonitor_title_delete_all_files",
            Self::UnmonitorShowDeleteExistingFiles => "unmonitor_show_delete_existing_files",
            Self::UnmonitorScopeDeleteFiles => "unmonitor_scope_delete_files",
            Self::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty => {
                "unmonitor_season_delete_files_then_delete_show_if_empty"
            }
            Self::UnmonitorSeasonThenUnmonitorShowIfEmpty => {
                "unmonitor_season_then_unmonitor_show_if_empty"
            }
            Self::ChangeQualityProfileAndSearchIfChanged => {
                "change_quality_profile_and_search_if_changed"
            }
            Self::AddTags => "add_tags",
            Self::RemoveTags => "remove_tags",
        }
    }

    /// Read a stored run's action back through the closed set. An
    /// unrecognized name is `None`: a row written by a newer build never
    /// collapses onto a fallback action here.
    pub fn parse_wire_str(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_wire_str() == value)
    }
}

/// The media subject a rule and its action are scoped to.
///
/// The source subject is preserved end to end: a season or episode match is
/// never widened into a title-level action.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceSubjectKind {
    Movie,
    Show,
    Season,
    Episode,
}

impl MaintenanceSubjectKind {
    /// Every subject kind.
    pub const ALL: &'static [Self] = &[Self::Movie, Self::Show, Self::Season, Self::Episode];
}

/// Risk class and its minimum control.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceRiskClass {
    /// No managed-media/catalog mutation; observe-only allowed.
    None,
    /// Reversible Scryer state, notification, or bounded refresh; normal arming
    /// and rate limit.
    Low,
    /// Changes acquisition intent, starts external work, or reorganizes files;
    /// affected-count preview and capability checks.
    Medium,
    /// Deletes, blocklists, or can make media unavailable; destructive
    /// confirmation, grace, current preview, cap, circuit breaker.
    High,
}

/// Effect classes every action step declares one or more of.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceEffectClass {
    Protect,
    Communicate,
    CatalogIntent,
    MetadataRepair,
    Acquisition,
    FileOrganization,
    DestructiveStorage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_kind_wire_names_are_pinned() {
        let expected = [
            (MaintenanceActionKind::DoNothing, "do_nothing"),
            (
                MaintenanceActionKind::UnmonitorScopeKeepFiles,
                "unmonitor_scope_keep_files",
            ),
            (
                MaintenanceActionKind::DeleteTitleAndFiles,
                "delete_title_and_files",
            ),
            (
                MaintenanceActionKind::UnmonitorTitleDeleteAllFiles,
                "unmonitor_title_delete_all_files",
            ),
            (
                MaintenanceActionKind::UnmonitorShowDeleteExistingFiles,
                "unmonitor_show_delete_existing_files",
            ),
            (
                MaintenanceActionKind::UnmonitorScopeDeleteFiles,
                "unmonitor_scope_delete_files",
            ),
            (
                MaintenanceActionKind::UnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty,
                "unmonitor_season_delete_files_then_delete_show_if_empty",
            ),
            (
                MaintenanceActionKind::UnmonitorSeasonThenUnmonitorShowIfEmpty,
                "unmonitor_season_then_unmonitor_show_if_empty",
            ),
            (
                MaintenanceActionKind::ChangeQualityProfileAndSearchIfChanged,
                "change_quality_profile_and_search_if_changed",
            ),
            (MaintenanceActionKind::AddTags, "add_tags"),
            (MaintenanceActionKind::RemoveTags, "remove_tags"),
        ];
        assert_eq!(expected.len(), MaintenanceActionKind::ALL.len());
        for (kind, wire) in expected {
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<MaintenanceActionKind>(&format!("\"{wire}\"")).unwrap(),
                kind
            );
            // A run row stores this string directly, so the hand-written
            // projection has to be the same name serde produces.
            assert_eq!(kind.as_wire_str(), wire);
            assert_eq!(MaintenanceActionKind::parse_wire_str(wire), Some(kind));
        }
        assert_eq!(MaintenanceActionKind::parse_wire_str("not_an_action"), None);
    }

    #[test]
    fn supporting_enum_wire_names_are_pinned() {
        for (subject, wire) in [
            (MaintenanceSubjectKind::Movie, "movie"),
            (MaintenanceSubjectKind::Show, "show"),
            (MaintenanceSubjectKind::Season, "season"),
            (MaintenanceSubjectKind::Episode, "episode"),
        ] {
            assert_eq!(
                serde_json::to_string(&subject).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (risk, wire) in [
            (MaintenanceRiskClass::None, "none"),
            (MaintenanceRiskClass::Low, "low"),
            (MaintenanceRiskClass::Medium, "medium"),
            (MaintenanceRiskClass::High, "high"),
        ] {
            assert_eq!(serde_json::to_string(&risk).unwrap(), format!("\"{wire}\""));
        }
        for (effect, wire) in [
            (MaintenanceEffectClass::Protect, "protect"),
            (MaintenanceEffectClass::Communicate, "communicate"),
            (MaintenanceEffectClass::CatalogIntent, "catalog_intent"),
            (MaintenanceEffectClass::MetadataRepair, "metadata_repair"),
            (MaintenanceEffectClass::Acquisition, "acquisition"),
            (
                MaintenanceEffectClass::FileOrganization,
                "file_organization",
            ),
            (
                MaintenanceEffectClass::DestructiveStorage,
                "destructive_storage",
            ),
        ] {
            assert_eq!(
                serde_json::to_string(&effect).unwrap(),
                format!("\"{wire}\"")
            );
        }
    }
}
