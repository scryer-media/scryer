// disk-guard: this crate is the media parser, one of the three crates clippy.toml exempts from the disk guard by name.
#![allow(clippy::disallowed_methods)]

//! Bounded media container and codec analysis.
//!
//! [`analyze_catalog_file`], [`analyze_file_with_options`] and
//! [`analyze_source`] identify the container ([`detect`]), parse it into raw
//! tracks ([`container`] and [`codec`]) and turn those into
//! [`AnalysisDetails`] ([`build`]). Every read goes through a byte and
//! operation budget ([`source::BoundedSource`]).

use std::io::{Read, Seek};
use std::path::Path;

mod build;
mod codec;
mod container;
mod detect;
pub mod diagnostics;
mod disc;
mod raw;
mod scan;
pub mod source;

#[cfg(test)]
pub(crate) use build::test_facts;
use container::{asf, audio_only, avi, dv, elementary, flv, mkv, mp4, ogg, ps, realmedia, ts};
use detect::{ContainerFormat, resolve_container_format, sniff_container_format_from_bytes};

/// Errors from container and codec parsing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum MediaInfoError {
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
}

impl From<std::io::Error> for MediaInfoError {
    fn from(e: std::io::Error) -> Self {
        MediaInfoError::Io(e.to_string())
    }
}

/// Analysis behavior profile for media probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisProfile {
    /// Minimal bounded analysis needed to identify a playable video file.
    /// This avoids optional chapter, frame-rate, HDR, and audio enrichment while
    /// retaining the bounded container and duration work needed for validation.
    ContentProbe,
    /// Fast bounded metadata pass. This avoids payload/sample deep probes and
    /// leaves richer confirmation to callers that need it.
    Fast,
    /// The full analysis, including bounded deep scans for metadata such as
    /// HDR10+ where cheaper signals justify them.
    DefaultRich,
    /// A stream and format pass with larger probe budgets when needed, plus a
    /// cheap first-frame HDR check for PQ video instead of the deeper bounded
    /// scans.
    FfprobeParity,
}

impl AnalysisProfile {
    pub(crate) fn skips_deep_probes(self) -> bool {
        matches!(self, Self::ContentProbe | Self::Fast)
    }
}

/// Options that control media analysis behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyzeOptions {
    pub profile: AnalysisProfile,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            profile: AnalysisProfile::Fast,
        }
    }
}

pub use scryer_media_model::AnalysisDetails;

/// Canonical analysis for import decisions, persisted metadata, and library scans.
/// Enrichment is bounded by each container parser and runs in the same pass as
/// metadata discovery so consumers do not reopen a file to fill missing facts.
pub fn analyze_catalog_file(file_path: &Path) -> Result<AnalysisDetails, MediaInfoError> {
    analyze_file_with_options(
        file_path,
        AnalyzeOptions {
            profile: AnalysisProfile::DefaultRich,
        },
    )
}

/// Analyze an intact disc image using a saved title choice and episode mappings.
/// A missing saved title remains unresolved and never falls back to automatic selection.
pub fn analyze_disc_file(
    file_path: &Path,
    selection: scryer_media_model::DiscSelection,
) -> Result<AnalysisDetails, MediaInfoError> {
    disc::analyze(file_path, selection, AnalysisProfile::DefaultRich)
}

/// Analyzes a media file with the requested analysis behavior profile.
pub fn analyze_file_with_options(
    file_path: &Path,
    options: AnalyzeOptions,
) -> Result<AnalysisDetails, MediaInfoError> {
    let ext = scryer_media_model::formats::canonical_video_extension(file_path)
        .map(str::to_owned)
        .or_else(|| {
            file_path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default();

    if scryer_media_model::formats::media_format_for_extension(&ext).is_some_and(|entry| {
        entry.handling == scryer_media_model::formats::MediaHandling::DiscImage
    }) {
        return disc::analyze(file_path, Default::default(), options.profile);
    }
    let mut source = match source::FileSource::open(file_path) {
        Ok(source) => source,
        // An unregistered suffix is unsupported whether or not the file is
        // readable; a registered suffix, or none at all, reports the I/O failure.
        Err(_)
            if !ext.is_empty()
                && scryer_media_model::formats::media_format_for_extension(&ext).is_none() =>
        {
            return Err(MediaInfoError::UnsupportedFormat(ext));
        }
        Err(error) => return Err(error.into()),
    };
    let details = analyze_source(&mut source, &ext, options)?;
    if !source.unchanged(file_path)? {
        return Err(MediaInfoError::Io(
            "media source changed during analysis".into(),
        ));
    }
    Ok(details)
}

/// Analyze a seekable logical media file, including a file assembled from disc extents.
/// The extension is only a hint; recognized container signatures take precedence.
pub fn analyze_source(
    input: &mut dyn source::MediaSource,
    extension: &str,
    options: AnalyzeOptions,
) -> Result<AnalysisDetails, MediaInfoError> {
    analyze_source_with_limits(input, extension, options, 64 * 1024 * 1024, 1024)
}

/// Analyze with explicit budgets, so exhaustion can be exercised without
/// movie-sized fixtures. Callers outside tests use [`analyze_source`].
pub(crate) fn analyze_source_with_limits(
    input: &mut dyn source::MediaSource,
    extension: &str,
    options: AnalyzeOptions,
    byte_budget: u64,
    io_limit: u64,
) -> Result<AnalysisDetails, MediaInfoError> {
    let started = std::time::Instant::now();
    let ext = extension.to_ascii_lowercase();
    let mut source = source::BoundedSource::new(input, byte_budget).with_io_limit(io_limit);
    source.seek(std::io::SeekFrom::Start(0))?;
    let mut header = [0_u8; 564];
    let size = source.read(&mut header)?;
    source.seek(std::io::SeekFrom::Start(0))?;
    let format = resolve_container_format(&ext, sniff_container_format_from_bytes(&header[..size]));

    let raw = match format {
        Some(ContainerFormat::Matroska) => {
            let profile = if options.profile == AnalysisProfile::Fast {
                AnalysisProfile::DefaultRich
            } else {
                options.profile
            };
            mkv::parse_mkv_source(&mut source, profile)
        }
        Some(ContainerFormat::Mp4) => mp4::parse_mp4_source(&mut source, &ext, options.profile),
        Some(ContainerFormat::Avi) => avi::parse_avi_source(&mut source, options.profile),
        Some(ContainerFormat::Ts) => ts::parse_ts_source(&mut source, options.profile),
        Some(ContainerFormat::Asf) => asf::parse_asf_source(&mut source),
        Some(ContainerFormat::Ogg) => ogg::parse_ogg_source(&mut source),
        Some(ContainerFormat::Flv) => flv::parse_flv_source(&mut source),
        Some(ContainerFormat::Ps) => ps::parse_ps(&mut source),
        Some(ContainerFormat::RealMedia) => realmedia::parse(&mut source),
        Some(ContainerFormat::Dv) => dv::parse(&mut source),
        Some(ContainerFormat::H264) => elementary::parse_h264(&mut source),
        Some(ContainerFormat::MpegVideo) => elementary::parse_mpeg(&mut source),
        Some(format @ (ContainerFormat::FlacAudio | ContainerFormat::WaveAudio)) => {
            audio_only::parse_audio_only(&mut source, format)
        }
        None => {
            if size == 0 || header.starts_with(b"MZ") || header.starts_with(b"\x7fELF") {
                return Err(MediaInfoError::Parse(
                    "empty or executable input is not media".into(),
                ));
            }
            return Err(MediaInfoError::UnsupportedFormat(ext));
        }
    };

    // Parsers stop enriching once the budget is gone, so a bounded failure here
    // means exhaustion struck before any track was parsed and there is nothing
    // to keep.
    let mut details = match raw {
        Ok(raw) => build::build_details(raw),
        Err(_) if source.exhausted => AnalysisDetails::default(),
        Err(error) => return Err(error),
    };
    details.revision = scryer_media_model::ANALYSIS_REVISION;
    let report = &mut details.report;
    report.bytes_read = source.bytes_read;
    report.seeks = source.seeks;
    report.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    report.budget_exhausted |= source.exhausted;
    if report.status == scryer_media_model::ProbeStatus::Unknown || source.exhausted {
        report.status = scryer_media_model::ProbeStatus::Incomplete;
    }
    if source.exhausted {
        report.warnings.push(scryer_media_model::ProbeWarning {
            code: "read_budget_exhausted".into(),
            message: "The aggregate media byte or I/O-operation budget was exhausted".into(),
            ..Default::default()
        });
    }
    Ok(details)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_media_is_distinct_from_empty_executable_and_io_failure() {
        use std::io::Cursor;
        for bytes in [
            Vec::new(),
            b"MZnot-media".to_vec(),
            b"\x7fELFnot-media".to_vec(),
        ] {
            assert!(matches!(
                analyze_source(&mut Cursor::new(bytes), "wtv", AnalyzeOptions::default()),
                Err(MediaInfoError::Parse(_))
            ));
        }
        assert!(matches!(
            analyze_source(
                &mut Cursor::new(b"legacy format"),
                "wtv",
                AnalyzeOptions::default()
            ),
            Err(MediaInfoError::UnsupportedFormat(_))
        ));
        assert!(matches!(
            analyze_catalog_file(Path::new("")),
            Err(MediaInfoError::Io(_))
        ));
    }
}
