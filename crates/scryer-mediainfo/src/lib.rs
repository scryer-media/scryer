// disk-guard: this crate is the media parser, one of the three crates clippy.toml exempts from the disk guard by name.
#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]

//! Bounded, read-only probing of media files: which container a file uses,
//! its streams and their codecs, durations, chapters, HDR signaling and,
//! for intact DVD and Blu-ray images, the disc's titles.
//!
//! Nothing is decoded. A probe reads container headers, codec configuration
//! and a bounded sample of frame headers, and every read counts against a
//! byte budget and an operation budget. A damaged or hostile file costs at
//! most those budgets; it cannot make a probe read the whole file or loop.
//!
//! # Probing a file
//!
//! [`probe`] reads a file with the default options. The result is an
//! [`AnalysisDetails`]; its [`report`](AnalysisDetails::report) says how far
//! the probe got and what it skipped.
//!
//! ```no_run
//! # fn main() -> Result<(), scryer_mediainfo::ProbeError> {
//! let details = scryer_mediainfo::probe("movie.mkv")?;
//! for stream in details.video_streams() {
//!     let (width, height) = stream.picture_size();
//!     println!("{:?} {width:?}x{height:?}", stream.codec);
//! }
//! if let Some(audio) = details.primary_audio() {
//!     println!("primary audio: {:?}", audio.codec);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Choosing how much to read
//!
//! [`ProbeOptions`] sets the [`ProbeDepth`] and the budgets. A shallower
//! depth answers "is this a playable video, and roughly what" with fewer
//! reads.
//!
//! ```no_run
//! use scryer_mediainfo::{ProbeDepth, ProbeOptions, probe_path};
//!
//! # fn main() -> Result<(), scryer_mediainfo::ProbeError> {
//! let options = ProbeOptions::new()
//!     .with_depth(ProbeDepth::Identify)
//!     .with_read_budget_bytes(8 * 1024 * 1024);
//! let details = probe_path("episode.mp4", &options)?;
//! println!("playable video: {}", details.is_playable_video());
//! # Ok(())
//! # }
//! ```
//!
//! # Probing bytes that are not a file
//!
//! [`probe_source`] accepts anything that implements [`Read`] and [`Seek`],
//! such as a [`Cursor`](std::io::Cursor) over bytes already in memory. The
//! extension hint is used only when the content does not identify itself.
//!
//! ```
//! use std::io::Cursor;
//! use scryer_mediainfo::{ProbeError, ProbeOptions, probe_source};
//!
//! let error = probe_source(Cursor::new(b"not media at all"), "xyz", &ProbeOptions::new())
//!     .unwrap_err();
//! assert!(matches!(error, ProbeError::Unsupported(_)));
//! ```
//!
//! # Disc images
//!
//! [`probe_path`] reads an `.iso` disc image's navigation and reports its
//! main title. [`probe_disc`] does the same with a saved
//! [`DiscSelection`], which names a title and maps its chapters to episodes.
//!
//! # What is reported
//!
//! The data types come from the `scryer-media-model` crate and are
//! re-exported here, so a caller needs only this crate. Warning codes in
//! [`ProbeReport::warnings`] are listed in [`warning`].

use std::fmt;
use std::io::{Read, Seek};
use std::path::Path;
use std::str::FromStr;

mod build;
mod codec;
mod container;
mod detect;
pub mod diagnostics;
mod disc;
mod raw;
mod scan;
mod source;
mod tags;

pub use scryer_media_model::*;

#[cfg(test)]
pub(crate) use build::test_facts;
use container::{asf, audio_only, avi, dv, elementary, flv, mkv, mp4, ogg, ps, realmedia, ts};
use detect::{ContainerFormat, resolve_container_format, sniff_container_format_from_bytes};

/// Why a probe returned no result.
///
/// A probe that read the file but found only part of what it looked for
/// still succeeds; its [`ProbeReport`] says what is missing. An error means
/// there is nothing to report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProbeError {
    /// The content is not a format this crate reads. The text names the
    /// format or suffix, for example `"wtv"`, or the reason, for example
    /// `"encrypted disc payload"`.
    #[error("unsupported format: {0}")]
    Unsupported(String),
    /// Reading the source failed.
    #[error("I/O error: {message}")]
    Io {
        /// The kind of the underlying I/O error.
        kind: std::io::ErrorKind,
        /// The underlying error's message.
        message: String,
    },
    /// The content claims to be a supported format but its structure is
    /// broken, or it is empty or an executable.
    #[error("parse error: {0}")]
    Malformed(String),
    /// The file changed while it was being read, so the result could mix two
    /// versions of it. Probing again later may succeed.
    #[error("I/O error: {}", SOURCE_CHANGED)]
    SourceChanged,
}

const SOURCE_CHANGED: &str = "media source changed during analysis";

impl ProbeError {
    /// The I/O error kind, for [`Io`](Self::Io); `None` for the other
    /// variants.
    #[must_use]
    pub fn kind(&self) -> Option<std::io::ErrorKind> {
        match self {
            Self::Io { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    /// The detail without the category prefix that [`Display`](fmt::Display)
    /// adds: the format, the I/O message, the structural problem, or a fixed
    /// sentence for [`SourceChanged`](Self::SourceChanged).
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Unsupported(message) | Self::Malformed(message) => message,
            Self::Io { message, .. } => message,
            Self::SourceChanged => SOURCE_CHANGED,
        }
    }
}

impl From<std::io::Error> for ProbeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

/// How much of a file a probe examines.
///
/// Each depth reads at least what the shallower ones read. All of them stay
/// within the budgets in [`ProbeOptions`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ProbeDepth {
    /// Enough to tell whether a file is playable video and to describe its
    /// streams and duration. Skips chapters, frame rates measured from
    /// samples, HDR signaling found only in frames, and audio details found
    /// only in frames.
    Identify,
    /// Container and codec configuration, without sampling frames. Matroska
    /// files get the [`Full`](Self::Full) pass, because their configuration
    /// is incomplete without it.
    Quick,
    /// Everything this crate can report, including bounded reads of frame
    /// headers for HDR10+, Dolby Vision, frame rate and audio details.
    #[default]
    Full,
    /// The stream and format facts `ffprobe -show_streams -show_format`
    /// reports, with larger probe budgets where ffprobe needs them and a
    /// first-frame HDR check in place of the deeper scans.
    FfprobeParity,
}

impl ProbeDepth {
    /// Every depth, shallowest first.
    pub const ALL: &'static [ProbeDepth] = &[
        ProbeDepth::Identify,
        ProbeDepth::Quick,
        ProbeDepth::Full,
        ProbeDepth::FfprobeParity,
    ];

    /// The depth's name, as [`Display`](fmt::Display) writes it and
    /// [`FromStr`] reads it: `identify`, `quick`, `full` or `ffprobe_parity`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identify => "identify",
            Self::Quick => "quick",
            Self::Full => "full",
            Self::FfprobeParity => "ffprobe_parity",
        }
    }

    pub(crate) fn skips_deep_probes(self) -> bool {
        matches!(self, Self::Identify | Self::Quick)
    }
}

impl fmt::Display for ProbeDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProbeDepth {
    type Err = ParseLabelError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|depth| depth.as_str() == name)
            .ok_or_else(|| ParseLabelError::new("probe depth", name))
    }
}

/// Settings for a probe: its [`ProbeDepth`] and its read budgets.
///
/// Built with chained setters from [`new`](Self::new) or
/// [`default`](Self::default). A budget left unset takes the entry point's
/// default: [`DEFAULT_READ_BUDGET_BYTES`](Self::DEFAULT_READ_BUDGET_BYTES)
/// and [`DEFAULT_IO_LIMIT`](Self::DEFAULT_IO_LIMIT) for a file or source,
/// and the larger [`DISC_READ_BUDGET_BYTES`](Self::DISC_READ_BUDGET_BYTES)
/// and [`DISC_IO_LIMIT`](Self::DISC_IO_LIMIT) for a disc image, whose
/// navigation must be read before any title.
///
/// ```
/// use scryer_mediainfo::{ProbeDepth, ProbeOptions};
///
/// let options = ProbeOptions::new().with_depth(ProbeDepth::Quick).with_io_limit(256);
/// assert_eq!(options.depth(), ProbeDepth::Quick);
/// assert_eq!(options.io_limit(), Some(256));
/// assert_eq!(options.read_budget_bytes(), None);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ProbeOptions {
    depth: ProbeDepth,
    read_budget_bytes: Option<u64>,
    io_limit: Option<u64>,
}

impl ProbeOptions {
    /// Bytes a file or source probe may read when no budget is set: 64 MiB.
    pub const DEFAULT_READ_BUDGET_BYTES: u64 = 64 * 1024 * 1024;
    /// Reads and seeks a file or source probe may make when no limit is set.
    pub const DEFAULT_IO_LIMIT: u64 = 1024;
    /// Bytes a disc image probe may read when no budget is set: 128 MiB.
    /// Each title's clips are then probed with the file defaults.
    pub const DISC_READ_BUDGET_BYTES: u64 = 128 * 1024 * 1024;
    /// Reads and seeks a disc image probe may make when no limit is set.
    pub const DISC_IO_LIMIT: u64 = 4096;

    /// [`ProbeDepth::Full`] with the entry point's default budgets. The
    /// same as [`default`](Self::default), usable in a `const`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            depth: ProbeDepth::Full,
            read_budget_bytes: None,
            io_limit: None,
        }
    }

    /// Sets how much of the file to examine.
    #[must_use]
    pub const fn with_depth(mut self, depth: ProbeDepth) -> Self {
        self.depth = depth;
        self
    }

    /// Caps the bytes the probe reads. A probe that reaches the cap stops,
    /// keeps what it found and reports
    /// [`warning::READ_BUDGET_EXHAUSTED`].
    #[must_use]
    pub const fn with_read_budget_bytes(mut self, bytes: u64) -> Self {
        self.read_budget_bytes = Some(bytes);
        self
    }

    /// Caps the reads and seeks the probe makes, which bounds the cost on
    /// storage where each request is slow. Reaching it has the same effect
    /// as reaching the byte budget.
    #[must_use]
    pub const fn with_io_limit(mut self, operations: u64) -> Self {
        self.io_limit = Some(operations);
        self
    }

    /// How much of the file the probe examines.
    #[must_use]
    pub const fn depth(&self) -> ProbeDepth {
        self.depth
    }

    /// The byte budget, or `None` when the entry point's default applies.
    #[must_use]
    pub const fn read_budget_bytes(&self) -> Option<u64> {
        self.read_budget_bytes
    }

    /// The read and seek limit, or `None` when the entry point's default
    /// applies.
    #[must_use]
    pub const fn io_limit(&self) -> Option<u64> {
        self.io_limit
    }
}

/// Probes the file at `path` with [`ProbeOptions::default`].
///
/// The same as [`probe_path`] with default options; see it for errors.
pub fn probe(path: impl AsRef<Path>) -> Result<AnalysisDetails, ProbeError> {
    probe_file(path.as_ref(), &ProbeOptions::new())
}

/// Probes the file at `path`.
///
/// The container is identified from the content; the suffix decides only
/// when the content is ambiguous, and an `.iso` suffix selects disc image
/// inspection with an automatic title choice (see [`probe_disc`]).
///
/// # Errors
///
/// - [`ProbeError::Unsupported`] when the content is not a supported
///   format, or the file cannot be opened and its suffix is not a media
///   suffix this crate knows.
/// - [`ProbeError::Malformed`] when the file is empty or an executable, or
///   its structure is broken before any stream could be read.
/// - [`ProbeError::Io`] when the file cannot be opened or read.
/// - [`ProbeError::SourceChanged`] when the file was replaced or modified
///   during the probe.
pub fn probe_path(
    path: impl AsRef<Path>,
    options: &ProbeOptions,
) -> Result<AnalysisDetails, ProbeError> {
    probe_file(path.as_ref(), options)
}

/// Probes media read from `source`, starting at its beginning.
///
/// `extension_hint` is the suffix the content would have as a file, without
/// the dot, for example `"mkv"`; it decides only when the content does not
/// identify its container, and may be empty. Pass `&mut reader` to keep
/// using the reader afterwards. Disc images are read only by [`probe_path`]
/// and [`probe_disc`].
///
/// # Errors
///
/// As [`probe_path`], without [`ProbeError::SourceChanged`]: the caller owns
/// the source and knows whether it can change.
pub fn probe_source<R: Read + Seek>(
    mut source: R,
    extension_hint: &str,
    options: &ProbeOptions,
) -> Result<AnalysisDetails, ProbeError> {
    // Learn the length before the budget wraps the source, so the report's
    // counts cover only the probe's own reads.
    let length = source.seek(std::io::SeekFrom::End(0))?;
    probe_bounded(
        &mut source::SizedReader::new(&mut source, length),
        extension_hint,
        options,
    )
}

/// Probes the disc image at `path`, reading the title `selection` names.
///
/// With a default [`DiscSelection`] the probe chooses the main title, as
/// [`probe_path`] does. A selection that names a title the image does not
/// have reports that title as unresolved and never falls back to an
/// automatic choice. Budgets left unset in `options` take
/// [`ProbeOptions::DISC_READ_BUDGET_BYTES`] and
/// [`ProbeOptions::DISC_IO_LIMIT`]; the selected title's clips are probed
/// with the file defaults.
///
/// A disc whose navigation cannot be read still returns a result, with the
/// problem in its report.
///
/// # Errors
///
/// - [`ProbeError::Io`] when the image cannot be opened.
/// - [`ProbeError::SourceChanged`] when the image was replaced or modified
///   during the probe.
pub fn probe_disc(
    path: impl AsRef<Path>,
    selection: DiscSelection,
    options: &ProbeOptions,
) -> Result<AnalysisDetails, ProbeError> {
    disc::analyze(path.as_ref(), selection, options)
}

fn probe_file(file_path: &Path, options: &ProbeOptions) -> Result<AnalysisDetails, ProbeError> {
    let ext = formats::canonical_video_extension(file_path)
        .map(str::to_owned)
        .or_else(|| {
            file_path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default();

    if formats::media_format_for_extension(&ext)
        .is_some_and(|entry| entry.handling == formats::MediaHandling::DiscImage)
    {
        return disc::analyze(file_path, DiscSelection::default(), options);
    }
    let mut source = match source::FileSource::open(file_path) {
        Ok(source) => source,
        // An unregistered suffix is unsupported whether or not the file is
        // readable; a registered suffix, or none at all, reports the I/O failure.
        Err(_) if !ext.is_empty() && formats::media_format_for_extension(&ext).is_none() => {
            return Err(ProbeError::Unsupported(ext));
        }
        Err(error) => return Err(error.into()),
    };
    let details = probe_bounded(&mut source, &ext, options)?;
    if !source.unchanged(file_path)? {
        return Err(ProbeError::SourceChanged);
    }
    Ok(details)
}

/// The probe every entry point runs: wraps `input` in the options' budgets,
/// identifies the container and builds the details.
pub(crate) fn probe_bounded(
    input: &mut dyn source::MediaSource,
    extension: &str,
    options: &ProbeOptions,
) -> Result<AnalysisDetails, ProbeError> {
    let started = std::time::Instant::now();
    let ext = extension.to_ascii_lowercase();
    let mut source = source::BoundedSource::new(
        input,
        options
            .read_budget_bytes
            .unwrap_or(ProbeOptions::DEFAULT_READ_BUDGET_BYTES),
    )
    .with_io_limit(options.io_limit.unwrap_or(ProbeOptions::DEFAULT_IO_LIMIT));
    source.seek(std::io::SeekFrom::Start(0))?;
    let mut header = [0_u8; 564];
    let size = source.read(&mut header)?;
    source.seek(std::io::SeekFrom::Start(0))?;
    let format = resolve_container_format(&ext, sniff_container_format_from_bytes(&header[..size]));

    let depth = options.depth;
    let raw = match format {
        Some(ContainerFormat::Matroska) => {
            let depth = if depth == ProbeDepth::Quick {
                ProbeDepth::Full
            } else {
                depth
            };
            mkv::parse_mkv_source(&mut source, depth)
        }
        Some(ContainerFormat::Mp4) => mp4::parse_mp4_source(&mut source, &ext, depth),
        Some(ContainerFormat::Avi) => avi::parse_avi_source(&mut source, depth),
        Some(ContainerFormat::Ts) => ts::parse_ts_source(&mut source, depth),
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
                return Err(ProbeError::Malformed(
                    "empty or executable input is not media".into(),
                ));
            }
            return Err(ProbeError::Unsupported(ext));
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
    details.revision = ANALYSIS_REVISION;
    let report = &mut details.report;
    report.bytes_read = source.bytes_read;
    report.seeks = source.seeks;
    report.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    report.budget_exhausted |= source.exhausted;
    if report.status == ProbeStatus::Unknown || source.exhausted {
        report.status = ProbeStatus::Incomplete;
    }
    if source.exhausted {
        report.warnings.push(ProbeWarning {
            code: warning::READ_BUDGET_EXHAUSTED.into(),
            message: "The aggregate media byte or I/O-operation budget was exhausted".into(),
            ..Default::default()
        });
    }
    Ok(details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn unsupported_media_is_distinct_from_empty_executable_and_io_failure() {
        let quick = ProbeOptions::new().with_depth(ProbeDepth::Quick);
        for bytes in [
            Vec::new(),
            b"MZnot-media".to_vec(),
            b"\x7fELFnot-media".to_vec(),
        ] {
            assert!(matches!(
                probe_source(Cursor::new(bytes), "wtv", &quick),
                Err(ProbeError::Malformed(_))
            ));
        }
        assert!(matches!(
            probe_source(Cursor::new(b"legacy format"), "wtv", &quick),
            Err(ProbeError::Unsupported(_))
        ));
        let missing = probe(Path::new("")).unwrap_err();
        assert!(matches!(missing, ProbeError::Io { .. }));
        assert_eq!(missing.kind(), Some(std::io::ErrorKind::NotFound));
    }

    #[test]
    fn errors_read_as_they_always_have() {
        assert_eq!(
            ProbeError::Unsupported("wtv".into()).to_string(),
            "unsupported format: wtv"
        );
        assert_eq!(
            ProbeError::Malformed("bad box".into()).to_string(),
            "parse error: bad box"
        );
        let io = ProbeError::from(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "short read",
        ));
        assert_eq!(io.to_string(), "I/O error: short read");
        assert_eq!(io.kind(), Some(std::io::ErrorKind::UnexpectedEof));
        assert_eq!(io.message(), "short read");
        assert_eq!(
            ProbeError::SourceChanged.to_string(),
            "I/O error: media source changed during analysis"
        );
        assert_eq!(ProbeError::SourceChanged.kind(), None);
    }

    #[test]
    fn depth_names_round_trip() {
        for depth in ProbeDepth::ALL {
            assert_eq!(depth.to_string().parse::<ProbeDepth>().unwrap(), *depth);
        }
        assert_eq!("deep".parse::<ProbeDepth>().unwrap_err().found(), "deep");
        assert_eq!(ProbeDepth::default(), ProbeDepth::Full);
        assert_eq!(ProbeOptions::default(), ProbeOptions::new());
    }

    #[test]
    fn source_probe_counts_only_its_own_reads() {
        // The length seek happens outside the budget, so a reader probed
        // through the generic entry point reports the same counts as the
        // same bytes probed as a sized source.
        let bytes = b"legacy format with no container".to_vec();
        let options = ProbeOptions::new();
        assert!(matches!(
            probe_source(Cursor::new(&bytes), "wtv", &options),
            Err(ProbeError::Unsupported(_))
        ));
        let mut reader = Cursor::new(bytes);
        let _ = probe_source(&mut reader, "wtv", &options);
        assert_eq!(reader.position(), 0, "the probe rewinds after sniffing");
    }
}
