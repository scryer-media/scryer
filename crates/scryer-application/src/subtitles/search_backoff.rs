//! "Searched, nothing found" backoff for automatic subtitle searches.
//!
//! When every provider answered an automatic search for one wanted language of
//! one media file and none of them had a usable subtitle, the file and language
//! are not searched automatically again until a growing delay has passed: 1
//! day, then 3, 7, 14, and every 30 days after that.
//!
//! A row stops applying when the file it describes changes (another path or
//! size on the media file row), when the wanted-language list changes, when
//! the set of providers asked for the file changes (one added, removed,
//! enabled, disabled, or moved to or from the file's facet), or when a
//! subtitle for that language is downloaded, which removes the row. A
//! minimum-score or other provider setting change leaves the row alone. A
//! search an operator starts ignores the delay. Rows go with their media file
//! (`ON DELETE CASCADE`), so nothing else prunes them.

use chrono::{DateTime, Duration, Utc};

use super::wanted::SubtitleLanguagePref;
use crate::TitleMediaFile;

/// Days to wait after the 1st, 2nd, 3rd, 4th, and every later consecutive
/// empty search.
const BACKOFF_DAYS: [i64; 5] = [1, 3, 7, 14, 30];

/// One file and language whose last automatic searches found nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleSearchBackoff {
    pub media_file_id: String,
    pub language: String,
    pub hearing_impaired: bool,
    pub forced: bool,
    /// The media file row's path when the search ran.
    pub file_path: String,
    /// The media file row's size when the search ran.
    pub size_bytes: i64,
    /// The wanted-language list the search ran under, as
    /// [`wanted_languages_key`] writes it.
    pub wanted_languages: String,
    /// The providers asked for the file's facet when the search ran, as
    /// [`enabled_providers_key`] writes it.
    pub enabled_providers: String,
    /// Consecutive automatic searches that found nothing.
    pub empty_searches: i32,
    /// RFC 3339 time of the last empty search.
    pub searched_at: String,
    /// RFC 3339 time before which no automatic search runs.
    pub retry_after: String,
}

/// The delay after `empty_searches` consecutive empty searches.
pub fn backoff_delay(empty_searches: i32) -> Duration {
    let index = usize::try_from(empty_searches.max(1) - 1)
        .unwrap_or(0)
        .min(BACKOFF_DAYS.len() - 1);
    Duration::days(BACKOFF_DAYS[index])
}

/// An order-independent key for the wanted-language list, so a row written
/// under one list stops applying once the list changes.
pub fn wanted_languages_key(languages: &[SubtitleLanguagePref]) -> String {
    let mut entries = languages
        .iter()
        .map(|pref| {
            format!(
                "{}:{}:{}",
                pref.code,
                u8::from(pref.hearing_impaired),
                u8::from(pref.forced)
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries.dedup();
    entries.join(",")
}

/// An order-independent key for the providers asked for one facet, by
/// provider config id, so a row written under one set stops applying once a
/// provider is added, removed, enabled, or disabled for that facet.
pub fn enabled_providers_key<'a>(config_ids: impl IntoIterator<Item = &'a str>) -> String {
    let mut ids = config_ids.into_iter().collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids.join(",")
}

impl SubtitleSearchBackoff {
    /// Whether this row describes `pref` for `file` under `wanted_key` and
    /// `providers_key`.
    pub fn applies_to(
        &self,
        file: &TitleMediaFile,
        pref: &SubtitleLanguagePref,
        wanted_key: &str,
        providers_key: &str,
    ) -> bool {
        self.media_file_id == file.id
            && self.language == pref.code
            && self.hearing_impaired == pref.hearing_impaired
            && self.forced == pref.forced
            && self.file_path == file.file_path
            && self.size_bytes == file.size_bytes
            && self.wanted_languages == wanted_key
            && self.enabled_providers == providers_key
    }

    /// Whether an automatic search must wait at `now`. A row whose retry time
    /// cannot be read does not hold the search back.
    pub fn holds_search_at(&self, now: DateTime<Utc>) -> bool {
        DateTime::parse_from_rfc3339(&self.retry_after)
            .is_ok_and(|retry_after| now < retry_after.with_timezone(&Utc))
    }

    /// The row an empty search at `now` leaves, given the row that applied
    /// before it (if any).
    pub fn after_empty_search(
        previous: Option<&SubtitleSearchBackoff>,
        file: &TitleMediaFile,
        pref: &SubtitleLanguagePref,
        wanted_key: &str,
        providers_key: &str,
        now: DateTime<Utc>,
    ) -> Self {
        let empty_searches = previous
            .map(|row| row.empty_searches.saturating_add(1))
            .unwrap_or(1);
        Self {
            media_file_id: file.id.clone(),
            language: pref.code.clone(),
            hearing_impaired: pref.hearing_impaired,
            forced: pref.forced,
            file_path: file.file_path.clone(),
            size_bytes: file.size_bytes,
            wanted_languages: wanted_key.to_string(),
            enabled_providers: providers_key.to_string(),
            empty_searches,
            searched_at: now.to_rfc3339(),
            retry_after: (now + backoff_delay(empty_searches)).to_rfc3339(),
        }
    }
}

/// The language identity of one row, as the store keys it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SubtitleSearchBackoffKey {
    pub language: String,
    pub hearing_impaired: bool,
    pub forced: bool,
}

impl From<&SubtitleLanguagePref> for SubtitleSearchBackoffKey {
    fn from(pref: &SubtitleLanguagePref) -> Self {
        Self {
            language: pref.code.clone(),
            hearing_impaired: pref.hearing_impaired,
            forced: pref.forced,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_grows_one_three_seven_fourteen_then_caps_at_thirty_days() {
        let days = (1..=8)
            .map(|count| backoff_delay(count).num_days())
            .collect::<Vec<_>>();
        assert_eq!(days, vec![1, 3, 7, 14, 30, 30, 30, 30]);
        assert_eq!(backoff_delay(i32::MAX).num_days(), 30);
    }

    #[test]
    fn enabled_providers_key_ignores_order_and_duplicates() {
        assert_eq!(
            enabled_providers_key(["ashgrove", "emberfall"]),
            enabled_providers_key(["emberfall", "ashgrove", "emberfall"]),
        );
        assert_ne!(
            enabled_providers_key(["emberfall"]),
            enabled_providers_key(["emberfall", "ashgrove"]),
        );
    }

    #[test]
    fn wanted_languages_key_ignores_order_and_tracks_flags() {
        let pref = |code: &str, hearing_impaired: bool, forced: bool| SubtitleLanguagePref {
            code: code.to_string(),
            hearing_impaired,
            forced,
        };
        assert_eq!(
            wanted_languages_key(&[pref("eng", false, false), pref("spa", true, false)]),
            wanted_languages_key(&[pref("spa", true, false), pref("eng", false, false)]),
        );
        assert_ne!(
            wanted_languages_key(&[pref("eng", false, false)]),
            wanted_languages_key(&[pref("eng", true, false)]),
        );
    }
}
