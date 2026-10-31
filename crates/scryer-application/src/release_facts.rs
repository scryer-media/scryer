//! Typed release facts read once from a host search result.
//!
//! Indexer plugins report torrent and identity facts through typed SDK fields
//! that the plugin host flattens into `IndexerSearchResult::extra`, and some
//! plugins set the same facts directly through `provider_extra`. This module is
//! the one reader of those keys, so every consumer (the client Nab feeds, the
//! GraphQL search payload) sees the same values.

use serde_json::Value;

use crate::IndexerSearchResult;

/// External ids an indexer asserted for a release. Values are trimmed and
/// non-empty; `imdb` is the bare numeric id zero padded to at least seven
/// digits, without the `tt` prefix.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseExternalIds {
    pub tvdb: Option<String>,
    pub tmdb: Option<String>,
    pub imdb: Option<String>,
    pub trakt: Option<String>,
    pub tvmaze: Option<String>,
    pub tvrage: Option<String>,
    pub douban: Option<String>,
}

/// Facts an indexer reported for one release. Counts are never negative and
/// factors and ratios are finite and never negative; a value that is absent or
/// fails those checks is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReleaseFacts {
    pub seeders: Option<i64>,
    pub leechers: Option<i64>,
    /// The reported peer count, or seeders plus leechers when the indexer
    /// reported both of those but no peer count.
    pub peers: Option<i64>,
    pub info_hash: Option<String>,
    pub magnet_url: Option<String>,
    pub download_volume_factor: Option<f64>,
    pub upload_volume_factor: Option<f64>,
    pub minimum_seed_ratio: Option<f64>,
    pub minimum_seed_time_minutes: Option<i64>,
    /// The indexer's explicit freeleech claim, or else whether the download
    /// volume factor is zero when a factor was reported.
    pub freeleech: Option<bool>,
    pub indexer_flags: Vec<String>,
    pub languages: Vec<String>,
    pub subtitles: Vec<String>,
    pub files: Option<i64>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub poster_url: Option<String>,
    pub external_ids: ReleaseExternalIds,
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn count(value: &Value) -> Option<i64> {
    match value {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
    .filter(|value| *value >= 0)
}

fn factor(value: &Value) -> Option<f64> {
    match value {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
    .filter(|value: &f64| value.is_finite() && *value >= 0.0)
}

fn flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        Value::Number(value) => value.as_i64().map(|value| value != 0),
        _ => None,
    }
}

fn texts(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(text).collect())
        .unwrap_or_default()
}

fn clean_list(values: Option<&Vec<String>>) -> Vec<String> {
    values
        .map(|values| {
            values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The bare IMDb number, zero padded to seven digits, or `None` when the value
/// is not an IMDb id.
fn normalize_imdb(value: &str) -> Option<String> {
    let value = value.trim();
    let digits = value
        .strip_prefix("tt")
        .or_else(|| value.strip_prefix("TT"))
        .unwrap_or(value);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| format!("{digits:0>7}"))
}

impl IndexerSearchResult {
    /// Read the typed release facts the plugin host recorded for this result.
    pub fn release_facts(&self) -> ReleaseFacts {
        let extra = |keys: &[&str]| keys.iter().find_map(|key| self.extra.get(*key));
        let first = |keys: &[&str], read: fn(&Value) -> Option<String>| {
            keys.iter()
                .filter_map(|key| self.extra.get(*key))
                .find_map(read)
        };
        let ids = self.extra.get("external_ids").and_then(Value::as_object);
        let id = |typed: Option<&String>, keys: &[&str]| {
            typed
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    keys.iter()
                        .filter_map(|key| ids.and_then(|ids| ids.get(*key)))
                        .find_map(text)
                })
                // Indexers report an unknown id as zero.
                .filter(|value| value != "0")
        };
        let attributes = &self.response_attributes;

        let seeders = extra(&["seeders"]).and_then(count);
        let leechers = extra(&["leechers"]).and_then(count);
        let peers = extra(&["peers"]).and_then(count).or_else(|| {
            seeders
                .zip(leechers)
                .and_then(|(seeders, leechers)| seeders.checked_add(leechers))
        });
        let download_volume_factor =
            extra(&["download_volume_factor", "downloadvolumefactor"]).and_then(factor);
        let upload_volume_factor =
            extra(&["upload_volume_factor", "uploadvolumefactor"]).and_then(factor);
        let freeleech = extra(&["freeleech"])
            .and_then(flag)
            .or_else(|| download_volume_factor.map(|value| value == 0.0));

        ReleaseFacts {
            seeders,
            leechers,
            peers,
            info_hash: first(&["info_hash", "info_hash_v1"], text),
            magnet_url: first(&["magnet_url", "magnet_uri"], text),
            download_volume_factor,
            upload_volume_factor,
            minimum_seed_ratio: extra(&["minimum_seed_ratio"]).and_then(factor),
            minimum_seed_time_minutes: extra(&["minimum_seed_time_minutes"]).and_then(count),
            freeleech,
            indexer_flags: texts(self.extra.get("indexer_flags")),
            languages: clean_list(self.indexer_languages.as_ref()),
            subtitles: clean_list(self.indexer_subtitles.as_ref()),
            files: extra(&["files"]).and_then(count),
            year: extra(&["year"])
                .and_then(count)
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| *value > 0),
            genre: first(&["genre"], text),
            poster_url: first(&["poster", "coverurl", "cover_url"], text),
            external_ids: ReleaseExternalIds {
                tvdb: id(attributes.tvdb_id.as_ref(), &["tvdb_id"]),
                tmdb: id(attributes.tmdb_id.as_ref(), &["tmdb_id"]),
                imdb: id(attributes.imdb_id.as_ref(), &["imdb_id"])
                    .as_deref()
                    .and_then(normalize_imdb),
                trakt: id(None, &["trakt_id", "trakt"]),
                tvmaze: id(None, &["tvmaze_id"]),
                tvrage: id(None, &["tvrage_id"]),
                douban: id(None, &["douban_id", "douban"]),
            },
        }
    }
}

#[cfg(test)]
#[path = "release_facts_tests.rs"]
mod tests;
