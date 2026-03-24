use crate::DownloadSourceKind;
use scryer_domain::MediaFacet;
use serde::{Deserialize, Serialize};

pub const DELAY_PROFILE_CATALOG_KEY: &str = "acquisition.delay_profiles";

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
    pub preferred_protocol: String,
    /// Usenet-only minimum age in minutes. Releases younger than this
    /// are held as pending regardless of score. 0 = disabled.
    #[serde(default)]
    pub min_age_minutes: i64,
    /// Score threshold to bypass delay for releases on the preferred protocol.
    #[serde(default)]
    pub bypass_score_threshold: Option<i32>,
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

fn default_preferred_protocol() -> String {
    "usenet".to_string()
}

pub fn parse_delay_profile_catalog(raw_json: &str) -> Result<Vec<DelayProfile>, serde_json::Error> {
    serde_json::from_str::<Vec<DelayProfile>>(raw_json)
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
    let facet_str = facet.as_str();

    let mut sorted: Vec<&DelayProfile> = profiles.iter().filter(|p| p.enabled).collect();
    sorted.sort_by_key(|p| p.priority);

    for profile in sorted {
        // Check facet filter
        if !profile.applies_to_facets.is_empty()
            && !profile
                .applies_to_facets
                .iter()
                .any(|f| f.eq_ignore_ascii_case(facet_str))
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

impl DelayProfile {
    /// Get the delay in minutes for the given source kind's protocol.
    pub fn get_protocol_delay(&self, source_kind: Option<DownloadSourceKind>) -> i64 {
        if is_usenet_source(source_kind) {
            self.usenet_delay_minutes
        } else {
            self.torrent_delay_minutes
        }
    }

    /// Whether the release is on the preferred protocol.
    pub fn is_preferred_protocol(&self, source_kind: Option<DownloadSourceKind>) -> bool {
        let usenet = is_usenet_source(source_kind);
        (usenet && self.preferred_protocol == "usenet")
            || (!usenet && self.preferred_protocol == "torrent")
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
        let delay = self.get_protocol_delay(source_kind);
        if delay <= 0 {
            return true;
        }
        if let Some(threshold) = self.bypass_score_threshold
            && candidate_score >= threshold
            && self.is_preferred_protocol(source_kind)
        {
            return true;
        }
        false
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

    fn make_profile(id: &str, priority: i32, usenet_delay: i64, torrent_delay: i64) -> DelayProfile {
        DelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            usenet_delay_minutes: usenet_delay,
            torrent_delay_minutes: torrent_delay,
            preferred_protocol: "usenet".to_string(),
            min_age_minutes: 0,
            bypass_score_threshold: None,
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
        profile.preferred_protocol = "usenet".to_string();

        // Usenet is preferred → bypass at threshold
        assert!(!profile.should_bypass_delay(Some(DownloadSourceKind::NzbFile), 1500));
        assert!(profile.should_bypass_delay(Some(DownloadSourceKind::NzbFile), 2000));

        // Torrent is NOT preferred → no bypass even at threshold
        assert!(!profile.should_bypass_delay(Some(DownloadSourceKind::TorrentFile), 3000));
    }

    #[test]
    fn protocol_delay_returns_correct_value() {
        let profile = make_profile("mixed", 10, 60, 360);
        assert_eq!(profile.get_protocol_delay(Some(DownloadSourceKind::NzbFile)), 60);
        assert_eq!(profile.get_protocol_delay(Some(DownloadSourceKind::NzbUrl)), 60);
        assert_eq!(profile.get_protocol_delay(Some(DownloadSourceKind::TorrentFile)), 360);
        assert_eq!(profile.get_protocol_delay(Some(DownloadSourceKind::MagnetUri)), 360);
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
}
