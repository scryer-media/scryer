use crate::DownloadSourceKind;
use chrono::{DateTime, Utc};
use scryer_domain::MediaFacet;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const DELAY_PROFILE_CATALOG_KEY: &str = "acquisition.delay_profiles";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PreferredProtocol {
    #[default]
    Usenet,
    Torrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DelayDecisionReason {
    #[default]
    Eligible,
    PendingDelay,
    MinimumAge,
    ReleaseAgeUnknown,
    ProtocolDisabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DelayDecision {
    pub reason: DelayDecisionReason,
    /// The exact publication-clock deadline. `delay_until` in persistence is
    /// only a derived cache of this value.
    pub eligible_at: Option<DateTime<Utc>>,
    pub min_age_hold_minutes: i64,
    pub protocol_hold_minutes: i64,
    pub effective_delay_minutes: i64,
}

impl DelayDecision {
    pub fn should_hold(&self) -> bool {
        matches!(
            self.reason,
            DelayDecisionReason::PendingDelay
                | DelayDecisionReason::MinimumAge
                | DelayDecisionReason::ReleaseAgeUnknown
        )
    }

    pub fn blocks_grab(&self) -> bool {
        self.reason != DelayDecisionReason::Eligible
    }
}

/// Facts supplied by the acquisition workflow for Sonarr-compatible delay
/// bypasses. The default context deliberately enables none of them.
#[derive(Debug, Clone, Copy, Default)]
pub struct DelayPolicyContext {
    pub user_invoked: bool,
    pub candidate_score: i32,
    pub preferred_protocol_same_tier_revision: bool,
    pub preferred_protocol_highest_quality: bool,
    pub oldest_overlapping_pending_published_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelayProfile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Delay for usenet releases (minutes). 0 = grab immediately.
    #[serde(default)]
    pub usenet_delay_minutes: i64,
    /// Delay for torrent releases (minutes). 0 = grab immediately.
    #[serde(default)]
    pub torrent_delay_minutes: i64,
    /// Preferred download protocol. Score-based bypass only applies
    /// when the release matches the preferred protocol.
    #[serde(default = "default_preferred_protocol")]
    pub preferred_protocol: PreferredProtocol,
    /// Usenet-only minimum age in minutes. Releases younger than this
    /// are held as pending regardless of score. 0 = disabled.
    #[serde(default)]
    pub min_age_minutes: i64,
    /// Score threshold to bypass delay for releases on the preferred protocol.
    #[serde(default)]
    pub bypass_score_threshold: Option<i32>,
    #[serde(default = "default_true")]
    pub enable_usenet: bool,
    #[serde(default = "default_true")]
    pub enable_torrent: bool,
    #[serde(default)]
    pub bypass_if_highest_quality: bool,
    #[serde(default)]
    pub applies_to_facets: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_preferred_protocol() -> PreferredProtocol {
    PreferredProtocol::Usenet
}

pub fn parse_delay_profile_catalog(raw_json: &str) -> Result<Vec<DelayProfile>, serde_json::Error> {
    serde_json::from_str::<Vec<DelayProfile>>(raw_json)
}

pub fn validate_delay_profile_catalog(profiles: &[DelayProfile]) -> Result<(), String> {
    let mut ids = HashSet::new();

    for profile in profiles {
        if profile.id.trim().is_empty() {
            return Err("delay profile id is required".to_string());
        }
        if !ids.insert(profile.id.to_ascii_lowercase()) {
            return Err(format!("duplicate delay profile id '{}'", profile.id));
        }
        if profile.name.trim().is_empty() {
            return Err(format!("delay profile '{}' must have a name", profile.id));
        }
        if profile.usenet_delay_minutes < 0 {
            return Err(format!(
                "delay profile '{}' has a negative usenet delay",
                profile.id
            ));
        }
        if profile.torrent_delay_minutes < 0 {
            return Err(format!(
                "delay profile '{}' has a negative torrent delay",
                profile.id
            ));
        }
        if profile.min_age_minutes < 0 {
            return Err(format!(
                "delay profile '{}' has a negative minimum usenet age",
                profile.id
            ));
        }
        if !profile.enable_usenet && !profile.enable_torrent {
            return Err(format!(
                "delay profile '{}' must enable at least one protocol",
                profile.id
            ));
        }
        if let Some(threshold) = profile.bypass_score_threshold
            && threshold < 0
        {
            return Err(format!(
                "delay profile '{}' has a negative bypass score threshold",
                profile.id
            ));
        }
        if let Some(invalid_facet) = profile
            .applies_to_facets
            .iter()
            .find(|facet| MediaFacet::parse(facet).is_none())
        {
            return Err(format!(
                "delay profile '{}' has an invalid facet '{}'",
                profile.id, invalid_facet
            ));
        }
    }

    Ok(())
}

/// Resolve the delay profile that applies to a title, based on tags and facet.
///
/// Profiles are evaluated in `priority ASC` order. A profile matches if:
/// 1. It is enabled.
/// 2. Its `tags` overlap with the title's tags, OR it has no tags (catch-all).
/// 3. Its `applies_to_facets` contains the title's facet, OR it has no facet filter.
///
/// Returns `None` if no profile matches — caller should grab immediately.
pub fn resolve_delay_profile<'a>(
    profiles: &'a [DelayProfile],
    title_tags: &[String],
    facet: &MediaFacet,
) -> Option<&'a DelayProfile> {
    let mut sorted: Vec<&DelayProfile> = profiles.iter().filter(|p| p.enabled).collect();
    sorted.sort_by_key(|p| p.priority);

    for profile in sorted {
        // Check facet filter
        if !profile.applies_to_facets.is_empty()
            && !profile
                .applies_to_facets
                .iter()
                .filter_map(|value| MediaFacet::parse(value))
                .any(|profile_facet| profile_facet == *facet)
        {
            continue;
        }

        // Check tag filter
        if !profile.tags.is_empty()
            && !profile
                .tags
                .iter()
                .any(|pt| title_tags.iter().any(|tt| tt.eq_ignore_ascii_case(pt)))
        {
            continue;
        }

        return Some(profile);
    }

    None
}

pub fn resolve_delay_decision(
    profiles: &[DelayProfile],
    title_tags: &[String],
    facet: &MediaFacet,
    source_kind: Option<DownloadSourceKind>,
    published_at: Option<DateTime<Utc>>,
    candidate_score: i32,
    now: &DateTime<Utc>,
) -> Option<DelayDecision> {
    let context = DelayPolicyContext {
        candidate_score,
        ..DelayPolicyContext::default()
    };
    resolve_delay_profile(profiles, title_tags, facet).map(|profile| {
        profile.evaluate_delay_decision_with_context(source_kind, published_at, &context, now)
    })
}

/// The one grab-time delay gate for every automatic acquisition lane.
///
/// Search evaluation uses this before it parks a candidate, and RSS, standby
/// recovery, and waiting-release promotion use it immediately before a grab.
/// Keeping those paths behind this wrapper prevents a policy edit from making
/// one of them observe a different delay decision than the others.
#[expect(
    clippy::too_many_arguments,
    reason = "the canonical grab-time check deliberately mirrors the shared delay decision inputs"
)]
pub(crate) fn grab_time_delay_decision(
    profiles: &[DelayProfile],
    title_tags: &[String],
    facet: &MediaFacet,
    source_kind: Option<DownloadSourceKind>,
    published_at: Option<DateTime<Utc>>,
    candidate_score: i32,
    _delay_started_at: Option<DateTime<Utc>>,
    now: &DateTime<Utc>,
) -> Option<DelayDecision> {
    grab_time_delay_decision_with_context(
        profiles,
        title_tags,
        facet,
        source_kind,
        published_at,
        &DelayPolicyContext {
            candidate_score,
            ..DelayPolicyContext::default()
        },
        now,
    )
}

pub(crate) fn grab_time_delay_decision_with_context(
    profiles: &[DelayProfile],
    title_tags: &[String],
    facet: &MediaFacet,
    source_kind: Option<DownloadSourceKind>,
    published_at: Option<DateTime<Utc>>,
    context: &DelayPolicyContext,
    now: &DateTime<Utc>,
) -> Option<DelayDecision> {
    resolve_delay_profile(profiles, title_tags, facet).map(|profile| {
        profile.evaluate_delay_decision_with_context(source_kind, published_at, context, now)
    })
}

impl DelayProfile {
    /// Get the delay in minutes for the given source kind's protocol.
    pub fn get_protocol_delay(&self, source_kind: Option<DownloadSourceKind>) -> i64 {
        if is_usenet_source(source_kind) {
            self.usenet_delay_minutes
        } else {
            self.torrent_delay_minutes
        }
    }

    pub fn is_protocol_enabled(&self, source_kind: Option<DownloadSourceKind>) -> bool {
        match source_kind {
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => self.enable_usenet,
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
                self.enable_torrent
            }
            None => true,
        }
    }

    /// Whether the release is on the preferred protocol.
    pub fn is_preferred_protocol(&self, source_kind: Option<DownloadSourceKind>) -> bool {
        match source_kind {
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => {
                self.preferred_protocol == PreferredProtocol::Usenet
            }
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
                self.preferred_protocol == PreferredProtocol::Torrent
            }
            None => false,
        }
    }

    /// Determine whether a release should bypass the protocol delay and be
    /// grabbed immediately.  Bypass happens when:
    /// - The protocol delay is 0 (no delay configured for this protocol), OR
    /// - The release is on the preferred protocol AND meets the score threshold.
    pub fn should_bypass_delay(
        &self,
        source_kind: Option<DownloadSourceKind>,
        candidate_score: i32,
    ) -> bool {
        self.should_bypass_protocol_delay(
            source_kind,
            &DelayPolicyContext {
                candidate_score,
                ..DelayPolicyContext::default()
            },
            &Utc::now(),
        )
    }

    pub fn release_age_unknown_escalation_deadline(
        &self,
        source_kind: Option<DownloadSourceKind>,
        first_seen_at: DateTime<Utc>,
    ) -> DateTime<Utc> {
        let min_age = if is_usenet_source(source_kind) {
            self.min_age_minutes.max(0)
        } else {
            0
        };
        first_seen_at
            + chrono::Duration::minutes(self.get_protocol_delay(source_kind).max(0).max(min_age))
    }

    pub fn evaluate_delay_decision(
        &self,
        source_kind: Option<DownloadSourceKind>,
        published_at: Option<DateTime<Utc>>,
        candidate_score: i32,
        now: &DateTime<Utc>,
    ) -> DelayDecision {
        self.evaluate_delay_decision_with_context(
            source_kind,
            published_at,
            &DelayPolicyContext {
                candidate_score,
                ..DelayPolicyContext::default()
            },
            now,
        )
    }

    pub fn evaluate_delay_decision_with_context(
        &self,
        source_kind: Option<DownloadSourceKind>,
        published_at: Option<DateTime<Utc>>,
        context: &DelayPolicyContext,
        now: &DateTime<Utc>,
    ) -> DelayDecision {
        if !self.is_protocol_enabled(source_kind) {
            return DelayDecision {
                reason: DelayDecisionReason::ProtocolDisabled,
                ..DelayDecision::default()
            };
        }

        let protocol_delay = self.get_protocol_delay(source_kind).max(0);
        let min_age_delay = if is_usenet_source(source_kind) {
            self.min_age_minutes.max(0)
        } else {
            0
        };
        let bypass_protocol_delay = self.should_bypass_protocol_delay(source_kind, context, now);
        let normal_delay_applies = protocol_delay > 0 && !bypass_protocol_delay;

        if published_at.is_none() && (min_age_delay > 0 || normal_delay_applies) {
            return DelayDecision {
                reason: DelayDecisionReason::ReleaseAgeUnknown,
                ..DelayDecision::default()
            };
        }

        let published_at = match published_at {
            Some(published_at) => published_at,
            None => return DelayDecision::default(),
        };
        let min_age_eligible_at =
            (min_age_delay > 0).then(|| published_at + chrono::Duration::minutes(min_age_delay));
        let protocol_eligible_at =
            normal_delay_applies.then(|| published_at + chrono::Duration::minutes(protocol_delay));
        let eligible_at = match (min_age_eligible_at, protocol_eligible_at) {
            (Some(minimum_age), Some(protocol_delay)) => Some(minimum_age.max(protocol_delay)),
            (Some(eligible_at), None) | (None, Some(eligible_at)) => Some(eligible_at),
            (None, None) => None,
        };
        let min_age_hold_minutes = min_age_eligible_at
            .map(|eligible_at| remaining_minutes(eligible_at, *now))
            .unwrap_or(0);
        let protocol_hold_minutes = protocol_eligible_at
            .map(|eligible_at| remaining_minutes(eligible_at, *now))
            .unwrap_or(0);
        let effective_delay_minutes = min_age_hold_minutes.max(protocol_hold_minutes);
        let reason = if protocol_hold_minutes > min_age_hold_minutes {
            DelayDecisionReason::PendingDelay
        } else if min_age_hold_minutes > 0 {
            DelayDecisionReason::MinimumAge
        } else if protocol_hold_minutes > 0 {
            DelayDecisionReason::PendingDelay
        } else {
            DelayDecisionReason::Eligible
        };

        DelayDecision {
            reason,
            eligible_at,
            min_age_hold_minutes,
            protocol_hold_minutes,
            effective_delay_minutes,
        }
    }

    fn should_bypass_protocol_delay(
        &self,
        source_kind: Option<DownloadSourceKind>,
        context: &DelayPolicyContext,
        now: &DateTime<Utc>,
    ) -> bool {
        if context.user_invoked {
            return true;
        }
        let delay = self.get_protocol_delay(source_kind).max(0);
        if delay == 0 {
            return true;
        }
        if self.is_preferred_protocol(source_kind)
            && (context.preferred_protocol_same_tier_revision
                || (self.bypass_if_highest_quality && context.preferred_protocol_highest_quality)
                || self
                    .bypass_score_threshold
                    .is_some_and(|threshold| context.candidate_score >= threshold))
        {
            return true;
        }
        context
            .oldest_overlapping_pending_published_at
            .is_some_and(|published_at| *now > published_at + chrono::Duration::minutes(delay))
    }
}

fn remaining_minutes(eligible_at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    if eligible_at <= now {
        0
    } else {
        ((eligible_at - now).num_seconds() + 59).max(1) / 60
    }
}

pub fn is_usenet_source(source_kind: Option<DownloadSourceKind>) -> bool {
    matches!(
        source_kind,
        Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile(
        id: &str,
        priority: i32,
        usenet_delay: i64,
        torrent_delay: i64,
    ) -> DelayProfile {
        DelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            usenet_delay_minutes: usenet_delay,
            torrent_delay_minutes: torrent_delay,
            preferred_protocol: PreferredProtocol::Usenet,
            min_age_minutes: 0,
            bypass_score_threshold: None,
            enable_usenet: true,
            enable_torrent: true,
            bypass_if_highest_quality: false,
            applies_to_facets: vec![],
            tags: vec![],
            priority,
            enabled: true,
        }
    }

    #[test]
    fn no_profiles_returns_none() {
        let result = resolve_delay_profile(&[], &[], &MediaFacet::Movie);
        assert!(result.is_none());
    }

    #[test]
    fn catch_all_profile_matches() {
        let profiles = vec![make_profile("default", 100, 360, 360)];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Movie);
        assert_eq!(result.unwrap().id, "default");
    }

    #[test]
    fn priority_ordering() {
        let profiles = vec![
            make_profile("low", 100, 720, 720),
            make_profile("high", 10, 360, 360),
        ];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Series);
        assert_eq!(result.unwrap().id, "high");
    }

    #[test]
    fn facet_filter_excludes() {
        let mut profile = make_profile("movies-only", 10, 360, 360);
        profile.applies_to_facets = vec!["movie".to_string()];
        let profiles = vec![profile];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Series);
        assert!(result.is_none());
    }

    #[test]
    fn facet_filter_includes() {
        let mut profile = make_profile("movies-only", 10, 360, 360);
        profile.applies_to_facets = vec!["movie".to_string()];
        let profiles = vec![profile];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Movie);
        assert_eq!(result.unwrap().id, "movies-only");
    }

    #[test]
    fn tag_filter_matches() {
        let mut profile = make_profile("tagged", 10, 360, 360);
        profile.tags = vec!["4k".to_string()];
        let catch_all = make_profile("default", 100, 720, 720);
        let profiles = vec![profile, catch_all];

        let result = resolve_delay_profile(
            &profiles,
            &["4k".to_string(), "anime".to_string()],
            &MediaFacet::Movie,
        );
        assert_eq!(result.unwrap().id, "tagged");
    }

    #[test]
    fn tag_filter_falls_through_to_catch_all() {
        let mut profile = make_profile("tagged", 10, 360, 360);
        profile.tags = vec!["4k".to_string()];
        let catch_all = make_profile("default", 100, 720, 720);
        let profiles = vec![profile, catch_all];

        let result = resolve_delay_profile(&profiles, &["hdr".to_string()], &MediaFacet::Movie);
        assert_eq!(result.unwrap().id, "default");
    }

    #[test]
    fn disabled_profile_skipped() {
        let mut profile = make_profile("disabled", 10, 360, 360);
        profile.enabled = false;
        let profiles = vec![profile];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Movie);
        assert!(result.is_none());
    }

    #[test]
    fn bypass_zero_usenet_delay() {
        let profile = make_profile("nodelay", 10, 0, 360);
        // Usenet delay is 0 → bypass for usenet
        assert!(profile.should_bypass_delay(Some(DownloadSourceKind::NzbFile), 500));
        // Torrent delay is 360 → no bypass without sufficient score
        assert!(!profile.should_bypass_delay(Some(DownloadSourceKind::TorrentFile), 500));
    }

    #[test]
    fn bypass_score_threshold_preferred_protocol() {
        let mut profile = make_profile("delayed", 10, 360, 360);
        profile.bypass_score_threshold = Some(2000);
        profile.preferred_protocol = PreferredProtocol::Usenet;

        // Usenet is preferred → bypass at threshold
        assert!(!profile.should_bypass_delay(Some(DownloadSourceKind::NzbFile), 1500));
        assert!(profile.should_bypass_delay(Some(DownloadSourceKind::NzbFile), 2000));

        // Torrent is NOT preferred → no bypass even at threshold
        assert!(!profile.should_bypass_delay(Some(DownloadSourceKind::TorrentFile), 3000));
    }

    #[test]
    fn protocol_delay_returns_correct_value() {
        let profile = make_profile("mixed", 10, 60, 360);
        assert_eq!(
            profile.get_protocol_delay(Some(DownloadSourceKind::NzbFile)),
            60
        );
        assert_eq!(
            profile.get_protocol_delay(Some(DownloadSourceKind::NzbUrl)),
            60
        );
        assert_eq!(
            profile.get_protocol_delay(Some(DownloadSourceKind::TorrentFile)),
            360
        );
        assert_eq!(
            profile.get_protocol_delay(Some(DownloadSourceKind::MagnetUri)),
            360
        );
        assert_eq!(profile.get_protocol_delay(None), 360); // default to torrent
    }

    #[test]
    fn parse_catalog_roundtrip() {
        let profiles = vec![make_profile("test", 10, 60, 360)];
        let json = serde_json::to_string(&profiles).unwrap();
        let parsed = parse_delay_profile_catalog(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "test");
        assert_eq!(parsed[0].usenet_delay_minutes, 60);
        assert_eq!(parsed[0].torrent_delay_minutes, 360);
    }

    #[test]
    fn legacy_profile_defaults_protocol_toggles() {
        let parsed = parse_delay_profile_catalog(r#"[{"id":"legacy","name":"Legacy"}]"#).unwrap();

        assert!(parsed[0].enable_usenet);
        assert!(parsed[0].enable_torrent);
        assert!(!parsed[0].bypass_if_highest_quality);
    }

    #[test]
    fn resolve_profile_accepts_series_facet_value() {
        let mut profile = make_profile("series", 10, 60, 360);
        profile.applies_to_facets = vec!["series".to_string()];
        let profiles = vec![profile];

        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Series);
        assert!(result.is_some());
    }

    #[test]
    fn old_release_is_immediately_eligible_from_its_publication_clock() {
        let profile = make_profile("test", 10, 60, 360);
        let now = Utc::now();
        let published_at = now - chrono::Duration::minutes(120);

        let decision = profile.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            100,
            &now,
        );

        assert_eq!(decision.reason, DelayDecisionReason::Eligible);
        assert_eq!(
            decision.eligible_at,
            Some(published_at + chrono::Duration::minutes(60))
        );
        assert!(!decision.blocks_grab());
    }

    #[test]
    fn fresh_release_is_held_until_the_exact_publication_deadline() {
        let profile = make_profile("test", 10, 60, 360);
        let now = Utc::now();
        let published_at = now - chrono::Duration::minutes(10);

        let decision = profile.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            100,
            &now,
        );

        assert_eq!(decision.reason, DelayDecisionReason::PendingDelay);
        assert_eq!(
            decision.eligible_at,
            Some(published_at + chrono::Duration::minutes(60))
        );
        assert_eq!(decision.protocol_hold_minutes, 50);
        assert!(decision.should_hold());
    }

    #[test]
    fn reobservation_does_not_restart_the_publication_clock() {
        let profile = make_profile("test", 10, 60, 360);
        let profiles = vec![profile];
        let now = Utc::now();
        let published_at = now - chrono::Duration::minutes(10);
        let decision = grab_time_delay_decision(
            &profiles,
            &[],
            &MediaFacet::Movie,
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            0,
            Some(now - chrono::Duration::minutes(1)),
            &now,
        )
        .unwrap();
        let reobserved = grab_time_delay_decision(
            &profiles,
            &[],
            &MediaFacet::Movie,
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            0,
            Some(now),
            &now,
        )
        .unwrap();

        assert_eq!(reobserved.eligible_at, decision.eligible_at);
        assert_eq!(
            reobserved.effective_delay_minutes,
            decision.effective_delay_minutes
        );
    }

    #[test]
    fn profile_edit_recomputes_from_the_original_publication_time() {
        let now = Utc::now();
        let published_at = now - chrono::Duration::minutes(10);
        let short_profile = make_profile("short", 10, 60, 360);
        let long_profile = make_profile("long", 10, 120, 360);

        let short = short_profile.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            0,
            &now,
        );
        let long = long_profile.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            0,
            &now,
        );

        assert_eq!(
            short.eligible_at,
            Some(published_at + chrono::Duration::minutes(60))
        );
        assert_eq!(
            long.eligible_at,
            Some(published_at + chrono::Duration::minutes(120))
        );
        assert_eq!(long.protocol_hold_minutes, 110);
    }

    #[test]
    fn future_publication_timestamp_remains_held() {
        let profile = make_profile("test", 10, 60, 360);
        let now = Utc::now();
        let published_at = now + chrono::Duration::minutes(30);

        let decision = profile.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            0,
            &now,
        );

        assert_eq!(decision.reason, DelayDecisionReason::PendingDelay);
        assert_eq!(
            decision.eligible_at,
            Some(now + chrono::Duration::minutes(90))
        );
        assert_eq!(decision.protocol_hold_minutes, 90);
    }

    #[test]
    fn active_age_gate_with_missing_timestamp_is_not_eligible() {
        let profile = make_profile("test", 10, 60, 360);
        let now = Utc::now();

        let decision =
            profile.evaluate_delay_decision(Some(DownloadSourceKind::NzbFile), None, 0, &now);

        assert_eq!(decision.reason, DelayDecisionReason::ReleaseAgeUnknown);
        assert!(decision.blocks_grab());
        assert!(decision.should_hold());
    }

    #[test]
    fn disabled_protocol_is_permanently_rejected() {
        let mut profile = make_profile("test", 10, 60, 360);
        profile.enable_usenet = false;

        let decision = profile.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(Utc::now() - chrono::Duration::hours(2)),
            0,
            &Utc::now(),
        );

        assert_eq!(decision.reason, DelayDecisionReason::ProtocolDisabled);
        assert!(decision.blocks_grab());
        assert!(!decision.should_hold());
    }

    #[test]
    fn minimum_age_is_hard_and_uses_the_later_deadline() {
        let mut profile = make_profile("test", 10, 60, 360);
        profile.min_age_minutes = 120;
        let now = Utc::now();
        let published_at = now - chrono::Duration::minutes(30);

        let decision = profile.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            100,
            &now,
        );

        assert_eq!(decision.reason, DelayDecisionReason::MinimumAge);
        assert_eq!(
            decision.eligible_at,
            Some(published_at + chrono::Duration::minutes(120))
        );
        assert_eq!(decision.min_age_hold_minutes, 90);
        assert_eq!(decision.protocol_hold_minutes, 30);
    }

    #[test]
    fn protocol_delay_reason_wins_when_its_deadline_is_later() {
        let mut profile = make_profile("test", 10, 180, 360);
        profile.min_age_minutes = 60;
        let now = Utc::now();
        let published_at = now - chrono::Duration::minutes(30);

        let decision = profile.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            100,
            &now,
        );

        assert_eq!(decision.reason, DelayDecisionReason::PendingDelay);
        assert_eq!(
            decision.eligible_at,
            Some(published_at + chrono::Duration::minutes(180))
        );
        assert_eq!(decision.min_age_hold_minutes, 30);
        assert_eq!(decision.protocol_hold_minutes, 150);
    }

    #[test]
    fn delay_bypasses_never_bypass_usenet_minimum_age() {
        let mut profile = make_profile("test", 10, 60, 360);
        profile.min_age_minutes = 120;
        profile.bypass_if_highest_quality = true;
        profile.bypass_score_threshold = Some(100);
        let now = Utc::now();
        let published_at = now - chrono::Duration::minutes(30);
        let bypasses = [
            DelayPolicyContext {
                user_invoked: true,
                ..DelayPolicyContext::default()
            },
            DelayPolicyContext {
                candidate_score: 100,
                ..DelayPolicyContext::default()
            },
            DelayPolicyContext {
                preferred_protocol_same_tier_revision: true,
                ..DelayPolicyContext::default()
            },
            DelayPolicyContext {
                preferred_protocol_highest_quality: true,
                ..DelayPolicyContext::default()
            },
            DelayPolicyContext {
                oldest_overlapping_pending_published_at: Some(now - chrono::Duration::minutes(61)),
                ..DelayPolicyContext::default()
            },
        ];

        for context in bypasses {
            let decision = profile.evaluate_delay_decision_with_context(
                Some(DownloadSourceKind::NzbFile),
                Some(published_at),
                &context,
                &now,
            );
            assert_eq!(decision.reason, DelayDecisionReason::MinimumAge);
            assert_eq!(
                decision.eligible_at,
                Some(published_at + chrono::Duration::minutes(120))
            );
        }

        let mut zero_delay = make_profile("zero", 10, 0, 360);
        zero_delay.min_age_minutes = 120;
        let decision = zero_delay.evaluate_delay_decision(
            Some(DownloadSourceKind::NzbFile),
            Some(published_at),
            0,
            &now,
        );
        assert_eq!(decision.reason, DelayDecisionReason::MinimumAge);
    }

    #[test]
    fn oldest_pending_and_escalation_deadline_use_profile_clock_values() {
        let mut profile = make_profile("test", 10, 60, 360);
        profile.min_age_minutes = 120;
        let now = Utc::now();
        let published_at = now - chrono::Duration::minutes(10);
        let context = DelayPolicyContext {
            oldest_overlapping_pending_published_at: Some(now - chrono::Duration::minutes(361)),
            ..DelayPolicyContext::default()
        };

        let decision = profile.evaluate_delay_decision_with_context(
            Some(DownloadSourceKind::TorrentFile),
            Some(published_at),
            &context,
            &now,
        );

        assert_eq!(decision.reason, DelayDecisionReason::Eligible);
        assert_eq!(
            profile
                .release_age_unknown_escalation_deadline(Some(DownloadSourceKind::NzbFile), now,),
            now + chrono::Duration::minutes(120)
        );
    }

    #[test]
    fn validation_rejects_duplicate_ids() {
        let profiles = vec![
            make_profile("dup", 10, 60, 60),
            make_profile("dup", 20, 120, 120),
        ];
        assert!(validate_delay_profile_catalog(&profiles).is_err());
    }

    #[test]
    fn validation_rejects_invalid_facet() {
        let mut profile = make_profile("bad", 10, 60, 60);
        profile.applies_to_facets = vec!["documentary".to_string()];
        let profiles = vec![profile];
        assert!(validate_delay_profile_catalog(&profiles).is_err());
    }

    #[test]
    fn validation_requires_at_least_one_enabled_protocol() {
        let mut profile = make_profile("disabled", 10, 60, 60);
        profile.enable_usenet = false;
        profile.enable_torrent = false;

        assert!(validate_delay_profile_catalog(&[profile]).is_err());
    }
}
