use crate::ParsedReleaseMetadata;

/// Build a dedup key from parsed release metadata for cross-indexer deduplication.
///
/// Two results with the same key are considered the same release from different
/// indexers. Returns an empty string if there's not enough metadata to build a
/// reliable key (in which case the result should be kept).
pub fn build_release_dedup_key(parsed: &ParsedReleaseMetadata) -> String {
    if parsed
        .parse_hints
        .iter()
        .any(|hint| hint.starts_with("stereo:") && hint.ends_with("_conflict"))
    {
        return String::new();
    }
    let group = parsed
        .release_group
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if group.is_empty() {
        return String::new();
    }

    let quality = parsed.quality.as_deref().unwrap_or("").to_ascii_lowercase();
    let codec = parsed
        .video_codec
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default()
        .to_ascii_lowercase();

    let episode_key = if let Some(ref ep) = parsed.episode {
        if let Some(air_date) = ep.air_date {
            format!("air{}", air_date.format("%Y-%m-%d"))
        } else if ep.release_type == crate::ParsedEpisodeReleaseType::SeasonPack {
            season_pack_key(ep)
        } else if let Some(season) = ep.season {
            let eps = ep
                .episode_numbers
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("s{season}e{eps}")
        } else if !ep.special_absolute_episode_numbers.is_empty() {
            format!(
                "special{}",
                ep.special_absolute_episode_numbers
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        } else if !ep.absolute_episode_numbers.is_empty() {
            format!(
                "abs{}",
                ep.absolute_episode_numbers
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        } else if let Some(abs) = ep.absolute_episode {
            format!("abs{abs}")
        } else {
            return String::new();
        }
    } else {
        return String::new();
    };

    let proper = if parsed.is_repack {
        "repack"
    } else if parsed.is_proper_upload {
        "proper"
    } else {
        ""
    };

    let dual = if parsed.is_dual_audio { "dual" } else { "" };
    let edition = parsed.edition.as_deref().unwrap_or("").to_ascii_lowercase();

    let base = format!("{group}|{episode_key}|{quality}|{codec}|{proper}|{dual}|{edition}");
    // Preserve the legacy key when no technical presentation was asserted.
    let Some(stereo) = parsed.stereoscopy else {
        return base;
    };
    format!(
        "{base}|stereo:{}:{}:{}:{}",
        stereo.presentation.as_str(),
        stereo.layout.map_or("", |value| value.as_str()),
        stereo.sampling.map_or("", |value| value.as_str()),
        stereo.encoding.map_or("", |value| value.as_str())
    )
}

/// Key of a season pack: the season, plus whatever makes the pack a
/// different set of episodes than the plain full-season pack. A part of a
/// season, a pack of several seasons, a complete-series pack and a season's
/// extras are each their own release, never a copy of the full-season pack.
/// A plain full-season pack keeps the bare `s{season}pack` form.
fn season_pack_key(ep: &crate::ParsedEpisodeMetadata) -> String {
    let mut key = format!("s{}pack", ep.season.unwrap_or(0));
    if ep.season_numbers.len() > 1 {
        let seasons = ep
            .season_numbers
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        key.push_str(&format!(":seasons{seasons}"));
    } else if ep.is_series_pack || ep.is_multi_season {
        key.push_str(":series");
    }
    if let Some(part) = ep.season_part {
        key.push_str(&format!(":part{part}"));
    } else if ep.is_partial_season {
        let eps = ep
            .episode_numbers
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        key.push_str(&format!(":partial{eps}"));
    }
    if ep.is_season_extra {
        key.push_str(":extras");
    }
    key
}

#[cfg(test)]
mod season_pack_key_tests {
    use super::*;
    use scryer_release_parser::{ContextFacetHint, ContextTitle, ReleaseParseContext};

    fn key(release: &str) -> String {
        let context = ReleaseParseContext {
            facet_hint: ContextFacetHint::Series,
            title: ContextTitle {
                name: "Glass Harbor".into(),
            },
            aliases: Vec::new(),
            known_years: Vec::new(),
            imdb_ids: Vec::new(),
            episodes: Vec::new(),
        };
        build_release_dedup_key(&crate::parse_release_metadata_for_target(release, &context))
    }

    #[test]
    fn the_same_full_season_pack_shares_one_key() {
        let full = key("Glass.Harbor.S02.1080p.WEB-DL.H.264-QUILLFOX");
        assert!(full.contains("|s2pack|"), "{full}");
        assert_eq!(full, key("Glass Harbor S02 1080p WEB-DL H.264-QUILLFOX"));
    }

    #[test]
    fn packs_holding_different_episodes_never_share_a_key() {
        let releases = [
            "Glass.Harbor.S02.1080p.WEB-DL.H.264-QUILLFOX",
            "Glass.Harbor.S02.Part.1.1080p.WEB-DL.H.264-QUILLFOX",
            "Glass.Harbor.S02.Part.2.1080p.WEB-DL.H.264-QUILLFOX",
            "Glass.Harbor.S02.Extras.1080p.WEB-DL.H.264-QUILLFOX",
            "Glass.Harbor.S02-S04.1080p.WEB-DL.H.264-QUILLFOX",
            "Glass.Harbor.S02-S03.1080p.WEB-DL.H.264-QUILLFOX",
        ];
        let mut seen = std::collections::HashMap::new();
        for release in releases {
            let key = key(release);
            assert!(!key.is_empty(), "{release} has no key");
            if let Some(other) = seen.insert(key.clone(), release) {
                panic!("{release} and {other} share the key {key}");
            }
        }
    }
}

#[cfg(test)]
mod stereo_tests {
    use super::*;
    use crate::{
        ParsedEpisodeMetadata, ParsedEpisodeReleaseType, ParsedStereoscopy, StereoEncoding,
        StereoLayout, StereoPresentation, StereoSampling,
    };

    #[test]
    fn stereo_variants_have_distinct_keys_and_conflicts_are_retained() {
        let mut parsed = ParsedReleaseMetadata {
            release_group: Some("Group".into()),
            episode: Some(ParsedEpisodeMetadata {
                season: Some(1),
                episode_numbers: vec![1],
                release_type: ParsedEpisodeReleaseType::SingleEpisode,
                ..Default::default()
            }),
            ..Default::default()
        };
        let unknown = build_release_dedup_key(&parsed);
        let mut keys = std::collections::HashSet::from([unknown]);
        for (presentation, layout, sampling, encoding) in [
            (StereoPresentation::TwoD, None, None, None),
            (StereoPresentation::ThreeD, None, None, None),
            (StereoPresentation::Mixed2d3d, None, None, None),
            (
                StereoPresentation::ThreeD,
                Some(StereoLayout::SideBySide),
                Some(StereoSampling::Half),
                None,
            ),
            (
                StereoPresentation::ThreeD,
                Some(StereoLayout::SideBySide),
                Some(StereoSampling::Full),
                None,
            ),
            (
                StereoPresentation::ThreeD,
                Some(StereoLayout::TopBottom),
                Some(StereoSampling::Half),
                None,
            ),
            (
                StereoPresentation::ThreeD,
                None,
                None,
                Some(StereoEncoding::Mvc),
            ),
        ] {
            parsed.stereoscopy = Some(ParsedStereoscopy {
                presentation,
                layout,
                sampling,
                encoding,
            });
            assert!(keys.insert(build_release_dedup_key(&parsed)));
        }
        parsed.parse_hints.push("stereo:layout_conflict".into());
        assert!(build_release_dedup_key(&parsed).is_empty());
        parsed.parse_hints.clear();
        parsed.episode = None;
        assert!(build_release_dedup_key(&parsed).is_empty());
    }
}
