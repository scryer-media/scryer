/// Parse a release title exactly as the grab path does for `title`.
///
/// Grab-time scoring (`catalog/release_search.rs`, RSS/auto search) parses every
/// candidate with `parse_release_metadata_for_target` against the title's
/// canonical parse context — name, aliases, year, external ids, facet — and
/// import must start from the same facts so the post-download score is the
/// grab score plus mediainfo deltas, not a context-free re-parse that can
/// disagree on quality, source, group, edition, or languages.
fn parse_import_release_for_title(
    release_title: &str,
    title: &scryer_domain::Title,
) -> ParsedReleaseMetadata {
    let evidence = crate::acquisition_release_search::canonical_title_evidence(title);
    crate::parse_release_metadata_for_target(release_title, &evidence.parse_context)
}

/// The quality the import will score for a file — the release evidence parsed
/// with the title's canonical context — for surfaces that must show it before
/// the import runs (the manual-import preview). Never the file name's own
/// tokens, which are not score evidence.
fn release_evidence_quality_for_title(
    source_video: &Path,
    release_evidence: &ReleaseEvidence,
    title: &scryer_domain::Title,
) -> Option<String> {
    release_evidence
        .release_title(Some(source_video))
        .and_then(|release_title| {
            normalize_release_title_signal(parse_import_release_for_title(&release_title, title))
                .quality
        })
}

/// Canonical import-time release metadata for a movie file: the release
/// evidence (never the downloader display label or destination folder) parsed
/// with the title's canonical grab-time context.
fn build_augmented_movie_import_metadata_for_title(
    source_video: &Path,
    release_evidence: &ReleaseEvidence,
    title: &scryer_domain::Title,
) -> ParsedReleaseMetadata {
    release_evidence
        .release_title(Some(source_video))
        .map(|release_title| {
            normalize_release_title_signal(parse_import_release_for_title(&release_title, title))
        })
        .unwrap_or_default()
}
