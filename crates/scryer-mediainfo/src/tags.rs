//! Bounded collection of the text tags containers attach to files and
//! streams. Tags come only from bytes a parser has already read for other
//! facts; nothing here reads the source.

use std::collections::BTreeMap;

use scryer_media_model::{AnalysisDetails, ProbeReport, ProbeWarning, warning};

/// Most tags kept for one file or one stream.
pub(crate) const MAX_TAGS_PER_SCOPE: usize = 256;
/// Longest key kept, in bytes.
pub(crate) const MAX_KEY_BYTES: usize = 256;
/// Longest value kept, in bytes.
pub(crate) const MAX_VALUE_BYTES: usize = 4096;
/// Most key and value bytes kept for a whole file, across every scope.
pub(crate) const MAX_FILE_BYTES: usize = 256 * 1024;

/// Adds one tag to `scope`. A tag that is empty, binary, not UTF-8 or a
/// repeat of a key already present is skipped silently. A tag past a limit
/// is dropped whole, never shortened, and the report gains one
/// `tags_truncated` warning.
pub(crate) fn insert(
    scope: &mut BTreeMap<String, String>,
    key: &[u8],
    value: &[u8],
    report: &mut ProbeReport,
) {
    let (Ok(key), Ok(value)) = (std::str::from_utf8(key), std::str::from_utf8(value)) else {
        return;
    };
    let value = value.trim_end_matches('\0');
    if key.is_empty() || value.is_empty() || !is_text(key) || !is_text(value) {
        return;
    }
    if scope.contains_key(key) {
        return;
    }
    if key.len() > MAX_KEY_BYTES
        || value.len() > MAX_VALUE_BYTES
        || scope.len() >= MAX_TAGS_PER_SCOPE
    {
        note_truncated(report);
        return;
    }
    scope.insert(key.to_owned(), value.to_owned());
}

/// Text, as opposed to binary data that happens to decode as UTF-8: no
/// control characters other than tab and line breaks.
fn is_text(text: &str) -> bool {
    !text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
}

fn note_truncated(report: &mut ProbeReport) {
    if report
        .warnings
        .iter()
        .any(|item| item.code == warning::TAGS_TRUNCATED)
    {
        return;
    }
    report.warnings.push(ProbeWarning {
        code: warning::TAGS_TRUNCATED.into(),
        message: "Some container tags were over a size limit and were left out".into(),
        ..Default::default()
    });
}

/// Holds the file's tags to [`MAX_FILE_BYTES`], keeping the file's own tags
/// first and then each stream's in stream order, and dropping whole tags
/// past the limit.
pub(crate) fn limit_file(details: &mut AnalysisDetails) {
    let mut budget = MAX_FILE_BYTES;
    let mut dropped = false;
    let mut keep = |scope: &mut BTreeMap<String, String>| {
        scope.retain(|key, value| {
            let size = key.len() + value.len();
            if size <= budget {
                budget -= size;
                true
            } else {
                dropped = true;
                false
            }
        });
    };
    keep(&mut details.tags);
    for stream in &mut details.streams {
        keep(&mut stream.metadata.tags);
    }
    if dropped {
        note_truncated(&mut details.report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&[u8], &[u8])]) -> (BTreeMap<String, String>, ProbeReport) {
        let mut scope = BTreeMap::new();
        let mut report = ProbeReport::default();
        for (key, value) in pairs {
            insert(&mut scope, key, value, &mut report);
        }
        (scope, report)
    }

    fn truncated(report: &ProbeReport) -> usize {
        report
            .warnings
            .iter()
            .filter(|item| item.code == warning::TAGS_TRUNCATED)
            .count()
    }

    #[test]
    fn keeps_text_and_skips_binary_empty_and_repeated_tags() {
        let (scope, report) = tags(&[
            (b"TITLE", b"Synthetic Feature"),
            (b"TITLE", b"Second Title"),
            (b"encoder", b"synthetic-muxer 1.0\0"),
            (b"COVER", &[0xff, 0xd8, 0xff]),
            (b"BLOB", &[0x00, 0x01, 0x02]),
            (b"EMPTY", b""),
            (b"", b"no key"),
        ]);
        assert_eq!(
            scope.into_iter().collect::<Vec<_>>(),
            [
                ("TITLE".to_owned(), "Synthetic Feature".to_owned()),
                ("encoder".to_owned(), "synthetic-muxer 1.0".to_owned()),
            ]
        );
        assert_eq!(truncated(&report), 0);
        assert_eq!(report.status, ProbeReport::default().status);
    }

    #[test]
    fn oversized_tags_are_dropped_whole_with_one_warning() {
        let long_value = vec![b'v'; MAX_VALUE_BYTES + 1];
        let long_key = vec![b'k'; MAX_KEY_BYTES + 1];
        let exact_value = vec![b'v'; MAX_VALUE_BYTES];
        let (scope, report) = tags(&[
            (b"LONG", &long_value),
            (&long_key, b"value"),
            (b"EXACT", &exact_value),
        ]);
        assert_eq!(scope.len(), 1);
        assert_eq!(scope["EXACT"].len(), MAX_VALUE_BYTES);
        assert_eq!(truncated(&report), 1);
        assert_eq!(report.status, ProbeReport::default().status);
    }

    #[test]
    fn a_scope_keeps_at_most_its_tag_count() {
        let keys: Vec<String> = (0..MAX_TAGS_PER_SCOPE + 10)
            .map(|index| format!("KEY{index:04}"))
            .collect();
        let mut scope = BTreeMap::new();
        let mut report = ProbeReport::default();
        for key in &keys {
            insert(&mut scope, key.as_bytes(), b"value", &mut report);
        }
        assert_eq!(scope.len(), MAX_TAGS_PER_SCOPE);
        assert!(scope.contains_key("KEY0000"));
        assert!(!scope.contains_key(&keys[MAX_TAGS_PER_SCOPE]));
        assert_eq!(truncated(&report), 1);
    }

    #[test]
    fn a_file_keeps_its_own_tags_before_its_streams() {
        let mut details = AnalysisDetails::default();
        let value = "v".repeat(MAX_VALUE_BYTES);
        for index in 0..40 {
            details
                .tags
                .insert(format!("FILE{index:02}"), value.clone());
        }
        let mut stream = scryer_media_model::StreamDetail::default();
        for index in 0..40 {
            stream
                .metadata
                .tags
                .insert(format!("STREAM{index:02}"), value.clone());
        }
        details.streams.push(stream);
        limit_file(&mut details);
        assert_eq!(details.tags.len(), 40);
        let kept = details.streams[0].metadata.tags.len();
        assert!(kept > 0 && kept < 40, "{kept}");
        let total: usize = details
            .tags
            .iter()
            .chain(&details.streams[0].metadata.tags)
            .map(|(key, value)| key.len() + value.len())
            .sum();
        assert!(total <= MAX_FILE_BYTES);
        assert_eq!(truncated(&details.report), 1);

        let mut small = AnalysisDetails::default();
        small.tags.insert("TITLE".into(), "Synthetic".into());
        limit_file(&mut small);
        assert_eq!(small.tags.len(), 1);
        assert!(small.report.warnings.is_empty());
    }
}
