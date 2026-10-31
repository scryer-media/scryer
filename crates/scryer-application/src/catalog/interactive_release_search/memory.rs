//! Shared accounting for retained raw-search payload and coordination storage.

use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{IndexerSearchResult, SubmissionScope};

pub(super) const RAW_SEARCH_RETAINED_BYTES: usize = 64 * 1024 * 1024;

/// One pool for all raw generations, including payload held by responses after
/// eviction. This is a retained-state bound, not a total process-memory claim.
pub(crate) struct RawSearchBudget {
    limit: usize,
    used: AtomicUsize,
}

impl Default for RawSearchBudget {
    fn default() -> Self {
        Self::new(RAW_SEARCH_RETAINED_BYTES)
    }
}

impl RawSearchBudget {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            used: AtomicUsize::new(0),
        }
    }

    pub(super) fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    pub(super) fn reserve(self: &Arc<Self>, bytes: usize) -> Option<MemoryCharge> {
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit)
            })
            .ok()?;
        Some(MemoryCharge {
            pool: self.clone(),
            bytes,
        })
    }
}

/// Allocation ownership, rather than registry membership, owns the charge.
pub(super) struct MemoryCharge {
    pool: Arc<RawSearchBudget>,
    bytes: usize,
}

impl MemoryCharge {
    pub(super) fn pool(&self) -> &Arc<RawSearchBudget> {
        &self.pool
    }

    pub(super) fn resize(&mut self, bytes: usize) -> bool {
        if bytes > self.bytes {
            let additional = bytes - self.bytes;
            if self
                .pool
                .used
                .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_add(additional)
                        .filter(|next| *next <= self.pool.limit)
                })
                .is_err()
            {
                return false;
            }
        } else {
            self.pool
                .used
                .fetch_sub(self.bytes - bytes, Ordering::AcqRel);
        }
        self.bytes = bytes;
        true
    }
}

impl Drop for MemoryCharge {
    fn drop(&mut self) {
        self.pool.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

fn optional(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, String::capacity)
}

fn strings(values: &Vec<String>) -> usize {
    values.capacity() * size_of::<String>() + values.iter().map(String::capacity).sum::<usize>()
}

/// Conservative hash-table storage including spare buckets, alignment and
/// control bytes. This also bounds compact IndexMap's entries plus index table
/// (serde_json uses the workspace's preserve_order feature). Nested allocations
/// are separate. Allocator tests verify the enabled, locked implementation.
pub(super) fn table_storage<K, V>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    capacity
        .checked_next_power_of_two()
        .unwrap_or(usize::MAX)
        .max(4)
        .saturating_mul(size_of::<(K, V)>().saturating_add(1))
        .saturating_add(64)
        .saturating_mul(2)
}

/// BTreeMap retains node slots rather than hash buckets; include a minimum
/// node and a conservative allowance for partially occupied internal nodes.
pub(super) fn tree_storage<K, V>(len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    len.max(11)
        .saturating_mul(size_of::<(K, V)>() + 2 * size_of::<usize>())
        .saturating_mul(3)
}

/// serde_json deliberately does not expose its object map's capacity. Rebuild
/// each object before admission so a removed large object cannot hide spare
/// buckets behind a small length. Keys, values, and insertion order survive.
/// Keep allocator tests in step with any future serde_json feature change.
pub(super) fn compact_json_objects(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                compact_json_objects(value);
            }
        }
        serde_json::Value::Object(values) => {
            let old = std::mem::take(values);
            let mut compact = serde_json::Map::with_capacity(old.len());
            for (key, mut value) in old {
                compact_json_objects(&mut value);
                compact.insert(key, value);
            }
            *values = compact;
        }
        _ => {}
    }
}

pub(super) fn json_heap(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 0,
        serde_json::Value::String(value) => value.capacity(),
        serde_json::Value::Array(values) => {
            values.capacity() * size_of::<serde_json::Value>()
                + values.iter().map(json_heap).sum::<usize>()
        }
        serde_json::Value::Object(values) => {
            table_storage::<String, serde_json::Value>(values.len())
                + values
                    .iter()
                    .map(|(key, value)| key.capacity() + json_heap(value))
                    .sum::<usize>()
        }
    }
}

fn scope_heap(scope: &SubmissionScope) -> usize {
    match scope {
        SubmissionScope::Episode { episode_id } => episode_id.capacity(),
        SubmissionScope::EpisodeSet { episode_ids } => strings(episode_ids),
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => series_movie_link_id.capacity(),
        SubmissionScope::Collection { collection_id } => collection_id.capacity(),
        SubmissionScope::Title | SubmissionScope::Orphan => 0,
    }
}

/// Allocation capacity, not logical string length or serialized byte length.
/// The fixed result storage is charged by its owning page's Vec capacity.
pub(super) fn result_heap(result: &IndexerSearchResult) -> usize {
    // Deliberately exhaustive: adding a field must revisit this accounting.
    let IndexerSearchResult {
        indexer_id: _,
        source: _,
        title: _,
        link: _,
        download_url: _,
        source_kind: _,
        size_bytes: _,
        published_at: _,
        thumbs_up: _,
        thumbs_down: _,
        indexer_languages: _,
        indexer_subtitles: _,
        indexer_grabs: _,
        password_hint: _,
        parsed_release_metadata: _,
        quality_profile_decision: _,
        extra: _,
        response_attributes: _,
        guid: _,
        info_url: _,
        provenance: _,
        candidate_token: _,
        queue_scope: _,
        coverage_scope: _,
        auto_eligible: _,
        auto_decision_code: _,
        auto_decision_summary: _,
        release_listing_json: _,
        http: _,
    } = result;
    let mut bytes = result.source.capacity() + result.title.capacity();
    for value in [
        &result.indexer_id,
        &result.link,
        &result.download_url,
        &result.published_at,
        &result.password_hint,
        &result.guid,
        &result.info_url,
        &result.candidate_token,
        &result.auto_decision_code,
        &result.auto_decision_summary,
        &result.release_listing_json,
    ] {
        bytes += optional(value);
    }
    for values in [&result.indexer_languages, &result.indexer_subtitles]
        .into_iter()
        .flatten()
    {
        bytes += strings(values);
    }
    bytes += table_storage::<String, serde_json::Value>(result.extra.capacity());
    bytes += result
        .extra
        .iter()
        .map(|(key, value)| key.capacity() + json_heap(value))
        .sum::<usize>();
    let attrs = &result.response_attributes;
    bytes += optional(&attrs.tvdb_id)
        + optional(&attrs.credential_parameter)
        + optional(&attrs.tmdb_id)
        + optional(&attrs.imdb_id)
        + strings(&attrs.categories)
        + attrs.original.capacity() * size_of::<scryer_plugin_sdk::PluginReleaseAttribute>();
    bytes += attrs
        .original
        .iter()
        .map(|attr| {
            attr.name.capacity()
                + attr.value.capacity()
                + optional(&attr.namespace)
                + optional(&attr.namespace_uri)
        })
        .sum::<usize>();
    for scope in [&result.queue_scope, &result.coverage_scope]
        .into_iter()
        .flatten()
    {
        bytes += scope_heap(scope);
    }
    if let Some(parsed) = &result.parsed_release_metadata {
        bytes += parsed.raw_title.capacity() + parsed.normalized_title.capacity();
        for values in [
            &parsed.normalized_tokens,
            &parsed.normalized_title_variants,
            &parsed.languages_audio,
            &parsed.languages_subtitles,
            &parsed.missing_fields,
            &parsed.parse_hints,
        ] {
            bytes += strings(values);
        }
        for value in [
            &parsed.release_group,
            &parsed.imdb_id,
            &parsed.tmdb_id,
            &parsed.tvdb_id,
            &parsed.quality,
            &parsed.video_encoding,
            &parsed.audio_channels,
            &parsed.edition,
        ] {
            bytes += optional(value);
        }
        bytes += parsed.external_ids.capacity()
            * size_of::<scryer_release_parser::ParsedExternalId>()
            + parsed
                .external_ids
                .iter()
                .map(|id| id.value.capacity())
                .sum::<usize>()
            + parsed.audio_codecs.capacity() * size_of::<scryer_release_parser::AudioCodec>();
        if let Some(episode) = &parsed.episode {
            bytes += optional(&episode.raw);
            for values in [
                &episode.season_numbers,
                &episode.episode_numbers,
                &episode.absolute_episode_numbers,
                &episode.special_absolute_episode_numbers,
            ] {
                bytes += values.capacity() * size_of::<u32>();
            }
        }
    }
    if let Some(decision) = &result.quality_profile_decision {
        bytes += strings(&decision.block_codes)
            + decision.scoring_log.capacity() * size_of::<crate::ScoringEntry>();
        for entry in &decision.scoring_log {
            bytes += entry.code.capacity();
            match &entry.source {
                crate::ScoringSource::Builtin => {}
                crate::ScoringSource::UserRule { id, name }
                | crate::ScoringSource::SystemRule { id, name } => {
                    bytes += id.capacity() + name.capacity();
                }
            }
        }
    }
    if let Some(http) = &result.http {
        bytes += http.provider.capacity() + http.item_id.capacity() + http.page_url.capacity();
        if let Some(channel) = &http.channel {
            bytes += channel.name.capacity() + optional(&channel.id);
        }
        if let Some(window) = &http.rights_window {
            bytes += optional(&window.starts_at) + optional(&window.ends_at);
        }
        if let Some(playlist) = &http.playlist {
            bytes += playlist.id.capacity();
        }
        if let Some(resolve) = &http.resolve {
            bytes += resolve.action.capacity();
        }
        bytes += http.resources.capacity() * size_of::<scryer_plugin_sdk::PluginHttpResource>();
        for resource in &http.resources {
            bytes += resource.url.capacity()
                + optional(&resource.expires_at)
                + optional(&resource.auth_ref)
                + optional(&resource.container)
                + optional(&resource.validators.etag)
                + optional(&resource.validators.last_modified)
                + optional(&resource.validators.checksum)
                + resource.headers.capacity() * size_of::<scryer_plugin_sdk::PluginHttpHeader>();
            bytes += resource
                .headers
                .iter()
                .map(|header| header.name.capacity() + header.value.capacity())
                .sum::<usize>();
            if let Some(video) = &resource.video {
                bytes += optional(&video.codec);
            }
            if let Some(audio) = &resource.audio {
                bytes += optional(&audio.codec) + optional(&audio.language);
            }
            if let Some(subtitle) = &resource.subtitle {
                bytes += optional(&subtitle.language) + optional(&subtitle.format);
            }
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_failure_preserves_accounting_and_drop_returns_capacity() {
        let pool = Arc::new(RawSearchBudget::new(100));
        let mut first = pool.reserve(60).unwrap();
        assert!(pool.reserve(41).is_none());
        assert!(!first.resize(101));
        assert_eq!(pool.used(), 60);
        let second = pool.reserve(40).unwrap();
        assert!(!first.resize(61));
        drop(second);
        assert!(first.resize(100));
        assert!(first.resize(20));
        assert_eq!(pool.used(), 20);
        drop(first);
        assert_eq!(pool.used(), 0);
    }

    #[test]
    fn json_accounting_includes_spare_capacity_and_nested_allocations() {
        let mut string = String::with_capacity(4096);
        string.push('x');
        let allocated = string.capacity();
        let mut values = Vec::with_capacity(128);
        values.push(serde_json::Value::String(string));
        let expected = allocated + values.capacity() * size_of::<serde_json::Value>();
        assert_eq!(json_heap(&serde_json::Value::Array(values)), expected);
    }
}
