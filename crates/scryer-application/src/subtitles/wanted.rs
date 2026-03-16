use scryer_domain::SubtitleDownload;

/// Represents a subtitle that is wanted but not yet downloaded.
#[derive(Debug, Clone)]
pub struct WantedSubtitle {
    pub media_file_id: String,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub language: String,
    pub hearing_impaired: bool,
    pub forced: bool,
    /// File path to the video on disk.
    pub video_path: String,
    /// Title name (for metadata-based search).
    pub title_name: String,
    /// Release year.
    pub year: Option<i32>,
    /// IMDb ID.
    pub imdb_id: Option<String>,
    /// Season number (series only).
    pub season: Option<i32>,
    /// Episode number (series only).
    pub episode: Option<i32>,
}

/// Language preference for subtitle downloading.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubtitleLanguagePref {
    /// ISO 639-2 code (e.g., "eng").
    pub code: String,
    /// Whether to specifically request hearing-impaired subs.
    pub hearing_impaired: bool,
    /// Whether to specifically request forced subs.
    pub forced: bool,
}

/// Determine which subtitles are missing for a set of media files.
///
/// Compares the wanted languages against:
/// 1. Already-downloaded external subtitles (from `subtitle_downloads` table)
/// 2. Embedded subtitle streams (from `media_files.subtitle_streams_json`)
///
/// Returns only the combinations that are still needed.
pub fn compute_missing_subtitles(
    wanted_languages: &[SubtitleLanguagePref],
    existing_downloads: &[SubtitleDownload],
    embedded_languages: &[String],
) -> Vec<SubtitleLanguagePref> {
    wanted_languages
        .iter()
        .filter(|want| {
            let lang = &want.code;

            // Check embedded streams
            if !want.forced && embedded_languages.iter().any(|e| e == lang) {
                return false;
            }

            // Check existing downloads
            !existing_downloads.iter().any(|dl| {
                dl.language == *lang
                    && dl.forced == want.forced
                    && dl.hearing_impaired == want.hearing_impaired
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_download(lang: &str, forced: bool, hi: bool) -> SubtitleDownload {
        SubtitleDownload {
            id: "test".into(),
            media_file_id: "mf1".into(),
            title_id: "t1".into(),
            episode_id: None,
            language: lang.into(),
            provider: "opensubtitles".into(),
            provider_file_id: None,
            file_path: "/tmp/test.srt".into(),
            score: Some(300),
            hearing_impaired: hi,
            forced,
            ai_translated: false,
            machine_translated: false,
            uploader: None,
            release_info: None,
            synced: false,
            downloaded_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn nothing_missing_when_all_downloaded() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: false,
            forced: false,
        }];
        let downloads = vec![make_download("eng", false, false)];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert!(missing.is_empty());
    }

    #[test]
    fn missing_when_no_downloads() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: false,
            forced: false,
        }];
        let missing = compute_missing_subtitles(&wanted, &[], &[]);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].code, "eng");
    }

    #[test]
    fn embedded_satisfies_non_forced() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: false,
            forced: false,
        }];
        let embedded = vec!["eng".to_string()];
        let missing = compute_missing_subtitles(&wanted, &[], &embedded);
        assert!(missing.is_empty());
    }

    #[test]
    fn embedded_does_not_satisfy_forced() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: false,
            forced: true,
        }];
        let embedded = vec!["eng".to_string()];
        let missing = compute_missing_subtitles(&wanted, &[], &embedded);
        assert_eq!(missing.len(), 1);
    }

    // ── Multiple languages, some satisfied ──────────────────────────

    #[test]
    fn multiple_languages_some_satisfied() {
        let wanted = vec![
            SubtitleLanguagePref {
                code: "eng".into(),
                hearing_impaired: false,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "spa".into(),
                hearing_impaired: false,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "fre".into(),
                hearing_impaired: false,
                forced: false,
            },
        ];
        let downloads = vec![make_download("eng", false, false)];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert_eq!(missing.len(), 2);
        let codes: Vec<&str> = missing.iter().map(|m| m.code.as_str()).collect();
        assert!(codes.contains(&"spa"));
        assert!(codes.contains(&"fre"));
        assert!(!codes.contains(&"eng"));
    }

    #[test]
    fn multiple_languages_all_satisfied() {
        let wanted = vec![
            SubtitleLanguagePref {
                code: "eng".into(),
                hearing_impaired: false,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "spa".into(),
                hearing_impaired: false,
                forced: false,
            },
        ];
        let downloads = vec![
            make_download("eng", false, false),
            make_download("spa", false, false),
        ];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert!(missing.is_empty());
    }

    #[test]
    fn multiple_languages_one_embedded_one_downloaded() {
        let wanted = vec![
            SubtitleLanguagePref {
                code: "eng".into(),
                hearing_impaired: false,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "spa".into(),
                hearing_impaired: false,
                forced: false,
            },
        ];
        let downloads = vec![make_download("spa", false, false)];
        let embedded = vec!["eng".to_string()];
        let missing = compute_missing_subtitles(&wanted, &downloads, &embedded);
        assert!(missing.is_empty());
    }

    // ── HI wanted but non-HI downloaded ─────────────────────────────

    #[test]
    fn hi_wanted_but_non_hi_downloaded_still_missing() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: true,
            forced: false,
        }];
        // Download has hearing_impaired = false
        let downloads = vec![make_download("eng", false, false)];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].hearing_impaired);
    }

    #[test]
    fn hi_wanted_and_hi_downloaded_is_satisfied() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: true,
            forced: false,
        }];
        let downloads = vec![make_download("eng", false, true)];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert!(missing.is_empty());
    }

    #[test]
    fn non_hi_wanted_but_hi_downloaded_still_missing() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: false,
            forced: false,
        }];
        // Download has hearing_impaired = true but we want non-HI
        let downloads = vec![make_download("eng", false, true)];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert_eq!(missing.len(), 1);
        assert!(!missing[0].hearing_impaired);
    }

    // ── Forced wanted, non-forced embedded ──────────────────────────

    #[test]
    fn forced_wanted_non_forced_embedded_still_missing() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: false,
            forced: true,
        }];
        // Embedded tracks are non-forced (embedded_languages are just language codes)
        let embedded = vec!["eng".to_string()];
        let missing = compute_missing_subtitles(&wanted, &[], &embedded);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].forced);
    }

    #[test]
    fn forced_wanted_non_forced_downloaded_still_missing() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: false,
            forced: true,
        }];
        // Download has forced = false
        let downloads = vec![make_download("eng", false, false)];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].forced);
    }

    #[test]
    fn forced_wanted_forced_downloaded_is_satisfied() {
        let wanted = vec![SubtitleLanguagePref {
            code: "eng".into(),
            hearing_impaired: false,
            forced: true,
        }];
        let downloads = vec![make_download("eng", true, false)];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert!(missing.is_empty());
    }

    // ── All satisfied by mix of embedded + downloads ────────────────

    #[test]
    fn all_satisfied_by_mix() {
        let wanted = vec![
            SubtitleLanguagePref {
                code: "eng".into(),
                hearing_impaired: false,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "spa".into(),
                hearing_impaired: false,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "fre".into(),
                hearing_impaired: true,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "jpn".into(),
                hearing_impaired: false,
                forced: true,
            },
        ];
        let downloads = vec![
            make_download("fre", false, true), // satisfies fre HI
            make_download("jpn", true, false), // satisfies jpn forced
        ];
        let embedded = vec!["eng".to_string(), "spa".to_string()];
        let missing = compute_missing_subtitles(&wanted, &downloads, &embedded);
        assert!(
            missing.is_empty(),
            "all should be satisfied, missing: {missing:?}"
        );
    }

    #[test]
    fn mix_of_satisfied_and_unsatisfied() {
        let wanted = vec![
            SubtitleLanguagePref {
                code: "eng".into(),
                hearing_impaired: false,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "spa".into(),
                hearing_impaired: true,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "fre".into(),
                hearing_impaired: false,
                forced: true,
            },
        ];
        let downloads = vec![make_download("eng", false, false)];
        let embedded = vec![];
        let missing = compute_missing_subtitles(&wanted, &downloads, &embedded);
        assert_eq!(missing.len(), 2);
        let codes: Vec<&str> = missing.iter().map(|m| m.code.as_str()).collect();
        assert!(codes.contains(&"spa"));
        assert!(codes.contains(&"fre"));
    }

    // ── Empty wanted list ───────────────────────────────────────────

    #[test]
    fn empty_wanted_returns_empty_missing() {
        let wanted: Vec<SubtitleLanguagePref> = vec![];
        let downloads = vec![make_download("eng", false, false)];
        let embedded = vec!["spa".to_string()];
        let missing = compute_missing_subtitles(&wanted, &downloads, &embedded);
        assert!(missing.is_empty());
    }

    #[test]
    fn empty_wanted_empty_downloads_returns_empty() {
        let missing = compute_missing_subtitles(&[], &[], &[]);
        assert!(missing.is_empty());
    }

    // ── Same language, different flags ───────────────────────────────

    #[test]
    fn same_language_different_flags_tracked_independently() {
        let wanted = vec![
            SubtitleLanguagePref {
                code: "eng".into(),
                hearing_impaired: false,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "eng".into(),
                hearing_impaired: true,
                forced: false,
            },
            SubtitleLanguagePref {
                code: "eng".into(),
                hearing_impaired: false,
                forced: true,
            },
        ];
        // Only the normal eng is downloaded
        let downloads = vec![make_download("eng", false, false)];
        let missing = compute_missing_subtitles(&wanted, &downloads, &[]);
        assert_eq!(missing.len(), 2);
    }
}
