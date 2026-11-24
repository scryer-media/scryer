//! Closed vocabularies shared by the lifecycle action sequence and history:
//! the subject a step acts on and the risk class that gates it.

use serde::{Deserialize, Serialize};

/// The media subject a rule and its action are scoped to.
///
/// The source subject is preserved end to end: a season or episode match is
/// never widened into a title-level action.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleSubjectKind {
    Movie,
    Show,
    Season,
    Episode,
}

impl LifecycleSubjectKind {
    /// Every subject kind.
    pub const ALL: &'static [Self] = &[Self::Movie, Self::Show, Self::Season, Self::Episode];
}

/// Risk class and its minimum control.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleRiskClass {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supporting_enum_wire_names_are_pinned() {
        for (subject, wire) in [
            (LifecycleSubjectKind::Movie, "movie"),
            (LifecycleSubjectKind::Show, "show"),
            (LifecycleSubjectKind::Season, "season"),
            (LifecycleSubjectKind::Episode, "episode"),
        ] {
            assert_eq!(
                serde_json::to_string(&subject).unwrap(),
                format!("\"{wire}\"")
            );
        }
        for (risk, wire) in [
            (LifecycleRiskClass::None, "none"),
            (LifecycleRiskClass::Low, "low"),
            (LifecycleRiskClass::Medium, "medium"),
            (LifecycleRiskClass::High, "high"),
        ] {
            assert_eq!(serde_json::to_string(&risk).unwrap(), format!("\"{wire}\""));
        }
    }
}
