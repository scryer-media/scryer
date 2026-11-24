//! Stable codes for [`ProbeWarning::code`](crate::ProbeWarning::code).
//!
//! A warning's `message` is for people and may be reworded; its `code` is
//! the stable identity to match on. Each constant below is one code the
//! parser can emit, and [`ALL`] lists every one of them. Codes are never
//! renamed or reused. A future version may add codes, so treat one you do
//! not recognize as a generic warning.
//!
//! ```
//! use scryer_media_model::{warning, ProbeReport};
//!
//! fn ran_out_of_budget(report: &ProbeReport) -> bool {
//!     report
//!         .warnings
//!         .iter()
//!         .any(|item| item.code == warning::READ_BUDGET_EXHAUSTED)
//! }
//! # assert!(!ran_out_of_budget(&ProbeReport::default()));
//! ```

macro_rules! warning_codes {
    ($($(#[doc = $doc:literal])* $name:ident = $value:literal;)*) => {
        $($(#[doc = $doc])* pub const $name: &str = $value;)*

        /// Every code above, in declaration order.
        pub const ALL: &[&str] = &[$($name,)*];
    };
}

warning_codes! {
    /// The probe stopped at its read budget or I/O operation limit, so later
    /// parts of the source were not read.
    READ_BUDGET_EXHAUSTED = "read_budget_exhausted";
    /// A stream's tag set, or the file's tag text as a whole, was over a size
    /// limit; the tags past the limit were left out.
    TAGS_TRUNCATED = "tags_truncated";

    /// A codec configuration record is absent where the codec needs one.
    AUDIO_CONFIGURATION_UNAVAILABLE = "audio_configuration_unavailable";
    /// A codec configuration record could not be read.
    AUDIO_CONFIGURATION_MALFORMED = "audio_configuration_malformed";
    /// An audio channel mapping uses a family the parser does not describe.
    AUDIO_CHANNEL_MAPPING_UNSUPPORTED = "audio_channel_mapping_unsupported";
    /// FLAC metadata blocks were incomplete or past the inspection limit.
    FLAC_METADATA_INCOMPLETE = "flac_metadata_incomplete";
    /// Container and bitstream color signaling disagree; the bitstream wins.
    COLOR_SIGNALING_CONFLICT = "color_signaling_conflict";
    /// AV1 header enrichment met incomplete, malformed or bounded data.
    AV1_ENRICHMENT_INCOMPLETE = "av1_enrichment_incomplete";
    /// A sequence parameter set was larger than the enrichment limit.
    SPS_ENRICHMENT_LIMIT = "sps_enrichment_limit";
    /// Video metadata sampling met an incomplete header or its inventory limit.
    VIDEO_METADATA_INCOMPLETE = "video_metadata_incomplete";
    /// Video enrichment stopped at its sample-count limit.
    VIDEO_ENRICHMENT_SAMPLE_LIMIT = "video_enrichment_sample_limit";
    /// Video enrichment stopped at its byte limit.
    VIDEO_ENRICHMENT_BYTE_LIMIT = "video_enrichment_byte_limit";
    /// Audio enrichment stopped at its sample-count limit.
    AUDIO_ENRICHMENT_SAMPLE_LIMIT = "audio_enrichment_sample_limit";
    /// Audio enrichment stopped at its byte limit.
    AUDIO_ENRICHMENT_BYTE_LIMIT = "audio_enrichment_byte_limit";
    /// Sample references needed for enrichment were missing or invalid.
    SAMPLE_REFERENCES_UNAVAILABLE = "sample_references_unavailable";
    /// A referenced sample could not be read in full.
    SAMPLE_READ_INCOMPLETE = "sample_read_incomplete";
    /// Presentation timing was sampled up to a limit rather than read in full.
    PRESENTATION_TIMING_SAMPLE_LIMIT = "presentation_timing_sample_limit";

    /// Matroska video enrichment stopped at its read budget.
    MKV_VIDEO_ENRICHMENT_BUDGET = "mkv_video_enrichment_budget";
    /// Matroska frame timing was observed from a bounded sample.
    MKV_FRAME_TIMING_SAMPLED = "mkv_frame_timing_sampled";
    /// The Matroska element inventory could not be completed.
    MKV_INVENTORY_INCOMPLETE = "mkv_inventory_incomplete";
    /// Matroska statistics tags disagree with the observed stream.
    MKV_STATISTICS_CONFLICT = "mkv_statistics_conflict";
    /// The supplemental Matroska inventory reached its coverage limit.
    MKV_INVENTORY_COVERAGE_LIMIT = "mkv_inventory_coverage_limit";

    /// An MP4 chapter track could not be read in full.
    MP4_CHAPTER_TRACK_INCOMPLETE = "mp4_chapter_track_incomplete";
    /// MP4 chapter markers could not be read in full.
    MP4_CHAPTERS_INCOMPLETE = "mp4_chapters_incomplete";
    /// The MP4 fragment inventory stopped at its budget.
    MP4_FRAGMENT_INVENTORY_BUDGET = "mp4_fragment_inventory_budget";
    /// The MP4 top-level box scan stopped at its budget.
    MP4_CONTAINER_SCAN_BUDGET = "mp4_container_scan_budget";

    /// ASF header extensions were nested past the inspection limit.
    ASF_HEADER_NESTING_LIMIT = "asf_header_nesting_limit";
    /// ASF pixel aspect attributes are incomplete, invalid or conflicting.
    ASF_ASPECT_INCOMPLETE = "asf_aspect_incomplete";
    /// ASF chapter markers could not be read in full.
    ASF_CHAPTERS_INCOMPLETE = "asf_chapters_incomplete";

    /// The AVI index read stopped at its read budget.
    AVI_INDEX_READ_BUDGET = "avi_index_read_budget";
    /// The AVI index was larger than the inspection budget.
    AVI_INDEX_BUDGET = "avi_index_budget";
    /// AVI sample inspection stopped at its budget.
    AVI_SAMPLE_BUDGET = "avi_sample_budget";
    /// An AVI index entry referenced a payload that could not be inspected.
    AVI_SAMPLE_REFERENCE_INVALID = "avi_sample_reference_invalid";

    /// The FLV tag scan stopped at its budget.
    FLV_TAG_SCAN_BUDGET = "flv_tag_scan_budget";

    /// An Ogg stream ends before the region the parser read at the file's end.
    OGG_EARLIER_STREAM_END_OUTSIDE_TAIL = "ogg_earlier_stream_end_outside_tail";

    /// The transport stream probe stopped at its read budget.
    TS_PROBE_READ_BUDGET = "ts_probe_read_budget";
    /// Transport stream enrichment stopped at its read budget.
    TS_STREAM_PROBE_BUDGET = "ts_stream_probe_budget";
    /// Transport stream enrichment ended before every requested fact was found.
    TS_STREAM_PROBE_INCOMPLETE = "ts_stream_probe_incomplete";
    /// An identify-depth duration was taken from timestamps without the final
    /// frame's duration.
    CONTENT_PROBE_DURATION_ESTIMATE = "content_probe_duration_estimate";

    /// A program stream carries timestamps that are out of range.
    PROGRAM_INVALID_TIMESTAMPS = "program_invalid_timestamps";
    /// Bounded sampling did not identify a program stream's codec.
    PROGRAM_CODEC_UNKNOWN = "program_codec_unknown";

    /// The disc image could not be inspected.
    DISC_INSPECTION = "disc_inspection";
    /// No valid disc title is selected and the saved selection needs review.
    DISC_SELECTION_REVIEW = "disc_selection_review";
    /// A saved episode mapping names a disc title that is missing or failed.
    DISC_EPISODE_MAPPING_REVIEW = "disc_episode_mapping_review";
    /// A disc title was inventoried but its media was not inspected.
    DISC_TITLE_NOT_SAMPLED = "disc_title_not_sampled";
    /// Disc and elementary-stream HDR signaling disagree; the bitstream wins.
    DISC_HDR_SIGNALING_CONFLICT = "disc_hdr_signaling_conflict";
    /// Clip declarations differ across the segments of one disc title.
    CLIP_DECLARATION_CHANGE = "clip_declaration_change";
    /// A playlist references a clip whose information file is missing.
    MISSING_CLIP_INFORMATION = "missing_clip_information";
    /// A playlist references a clip that is missing.
    MISSING_CLIP = "missing_clip";
    /// A referenced clip could not be inspected.
    CLIP_PROBE_FAILED = "clip_probe_failed";
    /// The disc's clip inspection budget ran out.
    CLIP_BUDGET = "clip_budget";
    /// A referenced clip ran out of its own analysis budget.
    CLIP_ENRICHMENT_BUDGET = "clip_enrichment_budget";
    /// A referenced clip's analysis was incomplete.
    CLIP_ANALYSIS_INCOMPLETE = "clip_analysis_incomplete";
    /// The segments of one disc title have materially different formats.
    SEGMENT_FORMAT_CHANGE = "segment_format_change";
    /// A Blu-ray playlist uses sub-paths, which are not followed.
    BLURAY_SUBPATHS = "bluray_subpaths";
    /// A Blu-ray playlist contains a still or interactive hold.
    BLURAY_STILL = "bluray_still";
    /// A DVD title needs navigation commands to be executed.
    DVD_DYNAMIC_NAVIGATION = "dvd_dynamic_navigation";
    /// A DVD title spans several program chains.
    DVD_MULTIPLE_PGCS = "dvd_multiple_pgcs";
    /// A DVD program chain is interactive.
    DVD_INTERACTIVE_PGC = "dvd_interactive_pgc";
    /// A DVD program chain carries navigation commands.
    DVD_PGC_COMMANDS = "dvd_pgc_commands";
    /// A DVD title uses interleaved angle blocks.
    DVD_INTERLEAVED_ANGLE = "dvd_interleaved_angle";
    /// A DVD cell needs a command or an interactive still.
    DVD_CELL_NAVIGATION = "dvd_cell_navigation";

    /// Structural diagnostics could not inspect the container.
    CONTAINER_INSPECTION = "container_inspection";
    /// Structural diagnostics could not cover the whole sampling plan.
    SAMPLING_INCOMPLETE = "sampling_incomplete";
    /// Structural diagnostics inspected bounded headers only, not decoded media.
    SAMPLED_HEADERS_ONLY = "sampled_headers_only";
    /// A transport packet is missing its sync byte.
    TRANSPORT_SYNC = "transport_sync";
    /// A transport packet has its transport-error indicator set.
    TRANSPORT_ERROR_INDICATOR = "transport_error_indicator";
    /// A transport packet is scrambled.
    TRANSPORT_SCRAMBLED = "transport_scrambled";
    /// A transport packet uses the reserved adaptation-field control value.
    TRANSPORT_ADAPTATION_CONTROL = "transport_adaptation_control";
    /// A transport packet's adaptation field has an invalid length.
    TRANSPORT_ADAPTATION_LENGTH = "transport_adaptation_length";
    /// A transport stream's continuity counter skipped.
    TRANSPORT_CONTINUITY_GAP = "transport_continuity_gap";
    /// A transport stream ends in a partial packet.
    TRANSPORT_TRUNCATION = "transport_truncation";
    /// A presentation timestamp has invalid flags, length or marker bits.
    PRESENTATION_TIMESTAMP_MARKERS = "presentation_timestamp_markers";
    /// Presentation timestamps go backwards.
    PRESENTATION_TIMESTAMP_REGRESSION = "presentation_timestamp_regression";
    /// A program stream pack header has an unrecognized version.
    PROGRAM_PACK_HEADER = "program_pack_header";
    /// A program stream ends in a partial packet.
    PROGRAM_TRUNCATION = "program_truncation";
    /// A program stream packet declares no length where one is required.
    PROGRAM_UNBOUNDED_PACKET = "program_unbounded_packet";
    /// A program stream packet is scrambled.
    PROGRAM_SCRAMBLED = "program_scrambled";
    /// A program stream PES header is invalid.
    PROGRAM_PES_HEADER = "program_pes_header";
    /// A program stream private-stream header is missing or truncated.
    PROGRAM_PRIVATE_HEADER = "program_private_header";
}

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn codes_are_distinct_snake_case() {
        let mut seen = std::collections::HashSet::new();
        for code in ALL {
            assert!(seen.insert(*code), "duplicate warning code {code}");
            assert!(!code.is_empty());
            assert!(
                code.bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
                "{code} is not snake_case"
            );
        }
    }
}
