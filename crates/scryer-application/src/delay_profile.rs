use scryer_domain::MediaFacet;
use serde::{Deserialize, Serialize};

pub const DELAY_PROFILE_CATALOG_KEY: &str = "acquisition.delay_profiles";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelayProfile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub delay_hours: i64,
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
    let facet_str = match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Tv => "tv",
        MediaFacet::Anime => "anime",
        MediaFacet::Other => "other",
    };

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

/// Determine whether a release should bypass the delay and be grabbed immediately.
pub fn should_bypass_delay(profile: &DelayProfile, candidate_score: i32) -> bool {
    if profile.delay_hours <= 0 {
        return true;
    }
    if let Some(threshold) = profile.bypass_score_threshold
        && candidate_score >= threshold
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_profile(id: &str, priority: i32, delay_hours: i64) -> DelayProfile {
        DelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            delay_hours,
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
        let profiles = vec![make_profile("default", 100, 6)];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Movie);
        assert_eq!(result.unwrap().id, "default");
    }

    #[test]
    fn priority_ordering() {
        let profiles = vec![make_profile("low", 100, 12), make_profile("high", 10, 6)];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Tv);
        assert_eq!(result.unwrap().id, "high");
    }

    #[test]
    fn facet_filter_excludes() {
        let mut profile = make_profile("movies-only", 10, 6);
        profile.applies_to_facets = vec!["movie".to_string()];
        let profiles = vec![profile];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Tv);
        assert!(result.is_none());
    }

    #[test]
    fn facet_filter_includes() {
        let mut profile = make_profile("movies-only", 10, 6);
        profile.applies_to_facets = vec!["movie".to_string()];
        let profiles = vec![profile];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Movie);
        assert_eq!(result.unwrap().id, "movies-only");
    }

    #[test]
    fn tag_filter_matches() {
        let mut profile = make_profile("tagged", 10, 6);
        profile.tags = vec!["4k".to_string()];
        let catch_all = make_profile("default", 100, 12);
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
        let mut profile = make_profile("tagged", 10, 6);
        profile.tags = vec!["4k".to_string()];
        let catch_all = make_profile("default", 100, 12);
        let profiles = vec![profile, catch_all];

        let result = resolve_delay_profile(&profiles, &["hdr".to_string()], &MediaFacet::Movie);
        assert_eq!(result.unwrap().id, "default");
    }

    #[test]
    fn disabled_profile_skipped() {
        let mut profile = make_profile("disabled", 10, 6);
        profile.enabled = false;
        let profiles = vec![profile];
        let result = resolve_delay_profile(&profiles, &[], &MediaFacet::Movie);
        assert!(result.is_none());
    }

    #[test]
    fn bypass_zero_delay() {
        let profile = make_profile("nodelay", 10, 0);
        assert!(should_bypass_delay(&profile, 500));
    }

    #[test]
    fn bypass_score_threshold() {
        let mut profile = make_profile("delayed", 10, 6);
        profile.bypass_score_threshold = Some(2000);
        assert!(!should_bypass_delay(&profile, 1500));
        assert!(should_bypass_delay(&profile, 2000));
        assert!(should_bypass_delay(&profile, 3000));
    }

    #[test]
    fn no_bypass_below_threshold() {
        let mut profile = make_profile("delayed", 10, 6);
        profile.bypass_score_threshold = Some(2000);
        assert!(!should_bypass_delay(&profile, 1999));
    }

    #[test]
    fn parse_catalog_roundtrip() {
        let profiles = vec![make_profile("test", 10, 6)];
        let json = serde_json::to_string(&profiles).unwrap();
        let parsed = parse_delay_profile_catalog(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "test");
        assert_eq!(parsed[0].delay_hours, 6);
    }
}
