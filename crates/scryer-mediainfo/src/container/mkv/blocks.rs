//! Bounded probes that read cluster blocks: frame rate, HDR10+, audio
//! profiles and the picture-header samples.

use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) struct BlockHeaderInfo {
    pub(super) track_number: u64,
    pub(super) timestamp: u64,
    pub(super) payload_offset: u64,
    pub(super) payload_size: u64,
    pub(super) lacing_type: u8,
    pub(super) invisible: bool,
}

#[derive(Debug, Default)]
pub(super) struct FrameRateProbeState {
    pub(super) timestamps: Vec<u64>,
    pub(super) matching_blocks: usize,
    pub(super) has_lacing: bool,
    pub(super) prefix_limit_reached: bool,
}

impl FrameRateProbeState {
    pub(super) fn done(&self) -> bool {
        self.timestamps.len() >= MKV_FPS_PROBE_MAX_TIMESTAMPS
            || self.matching_blocks >= MKV_FPS_PROBE_MAX_BLOCKS
            || self.prefix_limit_reached
    }

    pub(super) fn record_timestamp(&mut self, timestamp: u64) {
        self.matching_blocks += 1;
        self.timestamps.push(timestamp);
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Hdr10PlusProbeLimits {
    pub(super) max_bytes_read: u64,
    pub(super) max_video_blocks: usize,
    pub(super) max_file_scan_bytes: u64,
    pub(super) block_additional_peek_bytes: u64,
}

impl Hdr10PlusProbeLimits {
    pub(super) fn rich() -> Self {
        Self {
            max_bytes_read: MKV_RICH_HDR10PLUS_SCAN_MAX_BYTES,
            max_video_blocks: MKV_RICH_HDR10PLUS_MAX_VIDEO_BLOCKS,
            max_file_scan_bytes: u64::MAX,
            block_additional_peek_bytes: MKV_HDR10PLUS_BLOCKADDITIONAL_PEEK_BYTES,
        }
    }

    pub(super) fn first_frame() -> Self {
        Self {
            max_bytes_read: MKV_FIRST_FRAME_HDR10PLUS_SCAN_MAX_BYTES,
            max_video_blocks: MKV_FIRST_FRAME_HDR10PLUS_MAX_VIDEO_BLOCKS,
            max_file_scan_bytes: MKV_FIRST_FRAME_HDR10PLUS_MAX_FILE_SCAN_BYTES,
            block_additional_peek_bytes: MKV_HDR10PLUS_BLOCKADDITIONAL_PEEK_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Hdr10PlusProbeState {
    pub(super) nal_length_size: usize,
    pub(super) prefer_block_additional: bool,
    pub(super) limits: Hdr10PlusProbeLimits,
    pub(super) bytes_read: u64,
    pub(super) inspected_blocks: usize,
    pub(super) found: bool,
}

impl Hdr10PlusProbeState {
    pub(super) fn new(
        nal_length_size: usize,
        prefer_block_additional: bool,
        limits: Hdr10PlusProbeLimits,
    ) -> Self {
        Self {
            nal_length_size,
            prefer_block_additional,
            limits,
            bytes_read: 0,
            inspected_blocks: 0,
            found: false,
        }
    }

    pub(super) fn done(&self) -> bool {
        self.found
            || self.inspected_blocks >= self.limits.max_video_blocks
            || self.bytes_read >= self.limits.max_bytes_read
    }
}

#[derive(Debug, Default)]
pub(super) struct AudioProfileProbeState {
    pub(super) bytes_read: u64,
    pub(super) inspected_blocks: usize,
    pub(super) profile: Option<String>,
    pub(super) channels: Option<i32>,
    pub(super) dolby_header: Option<crate::codec::ac3::Ac3Header>,
    pub(super) mpeg_header: Option<crate::codec::mpeg_audio::MpegAudioHeader>,
    pub(super) dts_tentative_profile: Option<String>,
    pub(super) dts_tentative_hits: usize,
    pub(super) saw_dts_core: bool,
}

impl AudioProfileProbeState {
    pub(super) fn done(&self, codec_name: &str) -> bool {
        self.mpeg_header.is_some()
            || audio_profile_probe_is_terminal(codec_name, self.profile.as_deref())
            || self.inspected_blocks >= audio_profile_max_blocks(codec_name)
            || self.bytes_read >= MKV_AUDIO_PROFILE_SCAN_MAX_BYTES
    }

    pub(super) fn merge_profile(&mut self, codec_name: &str, candidate: Option<String>) {
        if codec_name != "dts" {
            merge_audio_profile(&mut self.profile, candidate);
            return;
        }

        match candidate.as_deref() {
            Some("DTS-HD MA") | Some("DTS-HD HRA") => {
                self.saw_dts_core = true;
                if self.dts_tentative_profile.as_deref() == candidate.as_deref() {
                    self.dts_tentative_hits += 1;
                } else {
                    self.dts_tentative_profile = candidate.clone();
                    self.dts_tentative_hits = 1;
                }
                if self.dts_tentative_hits >= 2 {
                    merge_audio_profile(&mut self.profile, candidate);
                }
            }
            Some("DTS")
            | Some("DTS-ES")
            | Some("DTS 96/24")
            | Some("DTS Express")
            | Some("DTS-HD MA + DTS:X")
            | Some("DTS-HD MA + DTS:X IMAX") => {
                self.saw_dts_core = true;
                merge_audio_profile(&mut self.profile, candidate);
            }
            Some(_) | None => {}
        }
    }

    pub(super) fn finalize(self, codec_name: &str) -> ScannedAudioMetadata {
        let profile = if codec_name == "dts"
            && self.profile.as_deref().is_none_or(|profile| {
                matches!(profile, "DTS" | "DTS-ES" | "DTS 96/24" | "DTS Express")
            })
            && self.saw_dts_core
        {
            self.profile.or_else(|| Some("DTS".into()))
        } else {
            self.profile
        };

        ScannedAudioMetadata {
            profile,
            channels: self.channels,
            dolby_header: self.dolby_header,
            mpeg_header: self.mpeg_header,
        }
    }
}

pub(super) fn audio_profile_max_blocks(codec_name: &str) -> usize {
    match codec_name {
        // Some AC-3 files start with stereo frames before the main 5.1 cadence.
        "ac3" => 8,
        // An MKV E-AC-3 stream's profile is the one its leading independent
        // syncframe declares. Scanning deeper promotes false positives from
        // later dependent or converted frames.
        "eac3" => MKV_EAC3_AUDIO_PROFILE_MAX_BLOCKS,
        _ => MKV_AUDIO_PROFILE_MAX_BLOCKS,
    }
}

pub(super) fn audio_profile_probe_is_terminal(codec_name: &str, profile: Option<&str>) -> bool {
    match (codec_name, profile) {
        ("dts", Some("DTS")) => false,
        (_, Some(_)) => true,
        _ => false,
    }
}

#[derive(Debug, Default)]
pub(super) struct ScannedAudioMetadata {
    pub(super) profile: Option<String>,
    pub(super) channels: Option<i32>,
    pub(super) dolby_header: Option<crate::codec::ac3::Ac3Header>,
    pub(super) mpeg_header: Option<crate::codec::mpeg_audio::MpegAudioHeader>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct MkvFrameRateProbeRequest {
    pub(super) target_track_num: u64,
    pub(super) timestamp_scale_ns: f64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct MkvHdr10PlusProbeRequest {
    pub(super) target_track_num: u64,
    pub(super) nal_length_size: usize,
    pub(super) prefer_block_additional: bool,
    pub(super) limits: Hdr10PlusProbeLimits,
}

impl MkvHdr10PlusProbeRequest {
    pub(super) fn new(
        target_track_num: u64,
        nal_length_size: usize,
        prefer_block_additional: bool,
        limits: Hdr10PlusProbeLimits,
    ) -> Self {
        Self {
            target_track_num,
            nal_length_size,
            prefer_block_additional,
            limits,
        }
    }
}

#[derive(Debug)]
pub(super) struct MkvAudioProbeRequest {
    pub(super) target_track_num: u64,
    pub(super) track_idx: usize,
    pub(super) codec_name: String,
    pub(super) header_strip_prefix: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
pub(super) struct MkvDeepProbeResult {
    pub(super) frame_rate_fps: Option<f64>,
    pub(super) frame_timing: Option<MkvFrameTiming>,
    pub(super) frame_timing_budget_exhausted: bool,
    pub(super) has_hdr10plus: Option<bool>,
    pub(super) audio: Vec<(usize, ScannedAudioMetadata)>,
}

pub(super) struct MkvFrameRateProbeTarget {
    pub(super) request: MkvFrameRateProbeRequest,
    pub(super) state: FrameRateProbeState,
}

pub(super) struct MkvHdr10PlusProbeTarget {
    pub(super) target_track_num: u64,
    pub(super) state: Hdr10PlusProbeState,
}

pub(super) struct MkvAudioProbeTarget {
    pub(super) target_track_num: u64,
    pub(super) track_idx: usize,
    pub(super) codec_name: String,
    pub(super) header_strip_prefix: Option<Vec<u8>>,
    pub(super) state: AudioProfileProbeState,
}

pub(super) struct MkvDeepProbePlan {
    pub(super) frame_rate: Option<MkvFrameRateProbeTarget>,
    pub(super) hdr10plus: Option<MkvHdr10PlusProbeTarget>,
    pub(super) audio: Vec<MkvAudioProbeTarget>,
}

impl MkvDeepProbePlan {
    pub(super) fn new(
        frame_rate: Option<MkvFrameRateProbeRequest>,
        hdr10plus: Option<MkvHdr10PlusProbeRequest>,
        audio: Vec<MkvAudioProbeRequest>,
    ) -> Self {
        Self {
            frame_rate: frame_rate.map(|request| MkvFrameRateProbeTarget {
                request,
                state: FrameRateProbeState::default(),
            }),
            hdr10plus: hdr10plus.map(|request| MkvHdr10PlusProbeTarget {
                target_track_num: request.target_track_num,
                state: Hdr10PlusProbeState::new(
                    request.nal_length_size,
                    request.prefer_block_additional,
                    request.limits,
                ),
            }),
            audio: audio
                .into_iter()
                .map(|request| MkvAudioProbeTarget {
                    target_track_num: request.target_track_num,
                    track_idx: request.track_idx,
                    codec_name: request.codec_name,
                    header_strip_prefix: request.header_strip_prefix,
                    state: AudioProfileProbeState::default(),
                })
                .collect(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.frame_rate.is_none() && self.hdr10plus.is_none() && self.audio.is_empty()
    }

    pub(super) fn done(&self) -> bool {
        self.frame_rate
            .as_ref()
            .is_none_or(|target| target.state.done())
            && self
                .hdr10plus
                .as_ref()
                .is_none_or(|target| target.state.done())
            && self
                .audio
                .iter()
                .all(|target| target.state.done(&target.codec_name))
    }

    pub(super) fn max_scan_end(&self, file_len: u64) -> u64 {
        let mut end = 0_u64;
        if self.frame_rate.is_some() {
            end = end.max(MKV_FPS_PROBE_MAX_SCAN_BYTES);
        }
        if let Some(target) = self.hdr10plus.as_ref() {
            end = end.max(target.state.limits.max_file_scan_bytes);
        }
        if !self.audio.is_empty() {
            end = end.max(MKV_AUDIO_PROFILE_MAX_FILE_SCAN_BYTES);
        }
        end.min(file_len)
    }

    pub(super) fn into_result(self) -> MkvDeepProbeResult {
        let frame_timing = self.frame_rate.as_ref().and_then(|target| {
            if target.state.has_lacing {
                return None;
            }
            observe_frame_timing(&target.state.timestamps, target.request.timestamp_scale_ns)
        });
        let frame_rate_fps = frame_timing
            .map(|timing| timing.rate.numerator as f64 / timing.rate.denominator as f64);
        let frame_timing_budget_exhausted = self
            .frame_rate
            .as_ref()
            .is_some_and(|target| target.state.done());
        let has_hdr10plus = self.hdr10plus.as_ref().map(|target| target.state.found);
        let audio = self
            .audio
            .into_iter()
            .filter_map(|target| {
                let scanned = target.state.finalize(&target.codec_name);
                (scanned.profile.is_some()
                    || scanned.channels.is_some()
                    || scanned.mpeg_header.is_some())
                .then_some((target.track_idx, scanned))
            })
            .collect();

        MkvDeepProbeResult {
            frame_rate_fps,
            frame_timing,
            frame_timing_budget_exhausted,
            has_hdr10plus,
            audio,
        }
    }
}

pub(super) struct MkvRawScanner<R> {
    pub(super) reader: R,
    pub(super) file_len: u64,
    pub(super) metadata_payload_bytes: u64,
    pub(super) pos: u64,
}

impl<R: Read + Seek> MkvRawScanner<R> {
    pub(super) fn new(mut reader: R) -> Result<Self, MediaInfoError> {
        let file_len = reader
            .seek(SeekFrom::End(0))
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        Self::new_with_file_len(reader, file_len)
    }

    pub(super) fn new_with_file_len(reader: R, file_len: u64) -> Result<Self, MediaInfoError> {
        Ok(Self {
            reader,
            file_len,
            metadata_payload_bytes: 0,
            pos: 0,
        })
    }

    pub(super) fn enrich_from_picture_header_samples(
        &mut self,
        end: u64,
        depth: usize,
        tracks: &mut [RawTrack],
        samples_left: &mut usize,
        report: &mut scryer_media_model::ProbeReport,
        captions: &mut Vec<scryer_media_model::CaptionService>,
    ) -> Result<(), MediaInfoError> {
        if depth > 10 || *samples_left == 0 {
            return Ok(());
        }
        while self.position()? < end && *samples_left > 0 {
            let Some(header) = self.read_element_header()? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            match header.id {
                EBML_ID_SEGMENT | EBML_ID_CLUSTER | EBML_ID_BLOCK_GROUP => self
                    .enrich_from_picture_header_samples(
                        child_end,
                        depth + 1,
                        tracks,
                        samples_left,
                        report,
                        captions,
                    )?,
                EBML_ID_SIMPLE_BLOCK | EBML_ID_BLOCK => {
                    let size = child_end.saturating_sub(self.position()?);
                    if let Some(block) = self.read_block_header(size, 32768)?
                        && let Some(track) = tracks.iter_mut().find(|track| {
                            track
                                .metadata
                                .id
                                .as_deref()
                                .and_then(|id| id.parse::<u64>().ok())
                                == Some(block.track_number)
                                && matches!(
                                    track.codec_name.as_deref(),
                                    Some(
                                        "h264"
                                            | "hevc"
                                            | "av1"
                                            | "vp8"
                                            | "vp9"
                                            | "mpeg1video"
                                            | "mpeg2video"
                                            | "mpeg4"
                                            | "vc1"
                                            | "mjpeg"
                                    )
                                )
                        })
                        && let Some((_, length)) = parse_first_laced_frame_from_reader(
                            self,
                            block.payload_size,
                            block.lacing_type,
                        )?
                    {
                        let bytes = self.read_bytes(length.min(64 * 1024))?;
                        match track.codec_name.as_deref() {
                            Some("av1") => crate::codec::av1::enrich(track, &bytes, report),
                            Some("h264" | "hevc") => {
                                let length_size = if track.codec_name.as_deref() == Some("h264") {
                                    track
                                        .codec_private
                                        .as_deref()
                                        .and_then(|bytes| bytes.get(4))
                                        .map_or(4, |byte| usize::from(byte & 3) + 1)
                                } else {
                                    track
                                        .codec_private
                                        .as_deref()
                                        .map(crate::codec::hevc::hevc_nal_length_size)
                                        .unwrap_or(4)
                                };
                                crate::codec::sei::length_prefixed(
                                    &bytes,
                                    length_size,
                                    track,
                                    report,
                                    captions,
                                );
                            }
                            _ => {
                                crate::codec::picture_headers::enrich(track, &bytes);
                                crate::codec::sei::annex_b(&bytes, track, report, captions);
                            }
                        }
                        *samples_left -= 1;
                    }
                }
                _ => {}
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    pub(super) fn read_next_segment_header(&mut self) -> Result<EbmlElementHeader, MediaInfoError> {
        while self.position()? < self.file_len {
            let Some(header) = self.read_element_header()? else {
                break;
            };
            let child_end = header.end(self.file_len, self.file_len);
            if header.id == EBML_ID_SEGMENT {
                return Ok(header);
            }
            self.seek_to(child_end)?;
        }
        Err(MediaInfoError::Parse("missing segment header".into()))
    }

    pub(super) fn run_deep_probe_plan(
        &mut self,
        plan: &mut MkvDeepProbePlan,
    ) -> Result<(), MediaInfoError> {
        if plan.is_empty() {
            return Ok(());
        }
        let end = plan.max_scan_end(self.file_len);
        self.seek_to(0)?;
        self.scan_root_for_deep_probe(end, plan, 0)?;
        if self.file_len > MKV_FPS_PROBE_MAX_SCAN_BYTES
            && self.position()? >= MKV_FPS_PROBE_MAX_SCAN_BYTES
            && let Some(target) = plan.frame_rate.as_mut()
        {
            target.state.prefix_limit_reached = true;
        }
        Ok(())
    }

    pub(super) fn scan_prefix_for_chapter_count(
        &mut self,
        max_bytes: u64,
    ) -> Result<Option<i32>, MediaInfoError> {
        let end = self.file_len.min(max_bytes);
        self.seek_to(0)?;
        while self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, &[EBML_ID_CHAPTERS])?
            else {
                return Ok(None);
            };
            let child_end = header.end(end, self.file_len);
            if header.id == EBML_ID_CHAPTERS
                && let Some(payload) = self.read_sized_payload(header, child_end)?
            {
                return Ok(parse_mkv_chapters_payload(&payload));
            }
            self.seek_to(child_end)?;
        }
        Ok(None)
    }

    pub(super) fn scan_prefix_for_audio_channels(
        &mut self,
        codec_name: &str,
        max_bytes: u64,
    ) -> Result<Option<i32>, MediaInfoError> {
        let size = self.file_len.min(max_bytes);
        if size == 0 {
            return Ok(None);
        }
        self.seek_to(0)?;
        let bytes = self.read_bytes(size)?;
        Ok(audio_channels_from_probe_bytes(codec_name, &bytes))
    }

    #[cfg(test)]
    pub(super) fn probe_frame_rate(
        &mut self,
        target_track_num: u64,
        timestamp_scale_ns: f64,
    ) -> Result<Option<f64>, MediaInfoError> {
        let mut plan = MkvDeepProbePlan::new(
            Some(MkvFrameRateProbeRequest {
                target_track_num,
                timestamp_scale_ns,
            }),
            None,
            Vec::new(),
        );
        self.run_deep_probe_plan(&mut plan)?;
        Ok(plan.into_result().frame_rate_fps)
    }

    pub(super) fn scan_root_for_deep_probe(
        &mut self,
        end: u64,
        plan: &mut MkvDeepProbePlan,
        depth: usize,
    ) -> Result<(), MediaInfoError> {
        const ROOT_IDS: &[u32] = &[EBML_ID_SEGMENT, EBML_ID_CLUSTER];
        while !plan.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, ROOT_IDS)? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            match header.id {
                EBML_ID_SEGMENT => {
                    if let Some(child_depth) = next_mkv_segment_probe_depth(depth) {
                        self.scan_root_for_deep_probe(child_end, plan, child_depth)?;
                    }
                }
                EBML_ID_CLUSTER => {
                    if let Some(child_depth) = next_mkv_deep_probe_depth(depth) {
                        self.scan_cluster_for_deep_probe(child_end, plan, child_depth)?;
                    }
                }
                _ => {}
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    pub(super) fn scan_cluster_for_deep_probe(
        &mut self,
        end: u64,
        plan: &mut MkvDeepProbePlan,
        depth: usize,
    ) -> Result<(), MediaInfoError> {
        const CLUSTER_IDS: &[u32] = &[EBML_ID_TIMESTAMP, EBML_ID_SIMPLE_BLOCK, EBML_ID_BLOCK_GROUP];
        let mut cluster_timestamp = 0_u64;
        while !plan.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, CLUSTER_IDS)? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            match header.id {
                EBML_ID_TIMESTAMP => {
                    if let Some(timestamp) =
                        self.read_unsigned_payload(child_end.saturating_sub(header.data_offset))?
                    {
                        cluster_timestamp = timestamp;
                    }
                }
                EBML_ID_SIMPLE_BLOCK => {
                    if let Some(block) = self.read_block_header(
                        child_end.saturating_sub(header.data_offset),
                        cluster_timestamp,
                    )? {
                        self.inspect_block_for_deep_probe(&block, plan)?;
                    }
                }
                EBML_ID_BLOCK_GROUP => {
                    if let Some(child_depth) = next_mkv_deep_probe_depth(depth) {
                        self.scan_block_group_for_deep_probe(
                            child_end,
                            cluster_timestamp,
                            plan,
                            child_depth,
                        )?;
                    }
                }
                _ => {}
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    pub(super) fn scan_block_group_for_deep_probe(
        &mut self,
        end: u64,
        cluster_timestamp: u64,
        plan: &mut MkvDeepProbePlan,
        depth: usize,
    ) -> Result<(), MediaInfoError> {
        const BLOCK_GROUP_IDS: &[u32] = &[EBML_ID_BLOCK, EBML_ID_BLOCK_ADDITIONS];
        let mut hdr10plus_target_block_in_group = false;
        while !plan.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, BLOCK_GROUP_IDS)? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            match header.id {
                EBML_ID_BLOCK => {
                    if let Some(block) = self.read_block_header(
                        child_end.saturating_sub(header.data_offset),
                        cluster_timestamp,
                    )? {
                        if let Some(target) = plan.hdr10plus.as_ref()
                            && target.state.prefer_block_additional
                            && block.track_number == target.target_track_num
                        {
                            hdr10plus_target_block_in_group = true;
                        }
                        self.inspect_block_for_deep_probe(&block, plan)?;
                    }
                }
                EBML_ID_BLOCK_ADDITIONS if hdr10plus_target_block_in_group => {
                    if let Some(child_depth) = next_mkv_deep_probe_depth(depth)
                        && let Some(target) = plan.hdr10plus.as_mut()
                        && target.state.prefer_block_additional
                    {
                        self.scan_block_additions_for_hdr10plus(
                            child_end,
                            &mut target.state,
                            child_depth,
                        )?;
                    }
                }
                _ => {}
            }
            self.seek_to(child_end)?;
        }
        if hdr10plus_target_block_in_group
            && let Some(target) = plan.hdr10plus.as_mut()
            && target.state.prefer_block_additional
            && target.state.inspected_blocks < target.state.limits.max_video_blocks
        {
            target.state.inspected_blocks += 1;
        }
        Ok(())
    }

    pub(super) fn inspect_block_for_deep_probe(
        &mut self,
        block: &BlockHeaderInfo,
        plan: &mut MkvDeepProbePlan,
    ) -> Result<(), MediaInfoError> {
        if let Some(target) = plan.frame_rate.as_mut()
            && !target.state.done()
            && block.track_number == target.request.target_track_num
        {
            if block.payload_offset >= MKV_FPS_PROBE_MAX_SCAN_BYTES {
                target.state.prefix_limit_reached = true;
            } else {
                target.state.has_lacing |= block.lacing_type != 0;
                if block.invisible {
                    target.state.matching_blocks += 1;
                } else {
                    target.state.record_timestamp(block.timestamp);
                }
            }
        }

        if let Some(target) = plan.hdr10plus.as_mut()
            && !target.state.done()
            && !target.state.prefer_block_additional
            && block.track_number == target.target_track_num
        {
            self.inspect_video_block_for_hdr10plus(block, &mut target.state)?;
        }

        for target in &mut plan.audio {
            if !target.state.done(&target.codec_name)
                && block.track_number == target.target_track_num
            {
                self.inspect_audio_block_for_profile(
                    block,
                    &target.codec_name,
                    target.header_strip_prefix.as_deref(),
                    &mut target.state,
                )?;
            }
        }

        Ok(())
    }

    pub(super) fn scan_block_additions_for_hdr10plus(
        &mut self,
        end: u64,
        state: &mut Hdr10PlusProbeState,
        depth: usize,
    ) -> Result<(), MediaInfoError> {
        const BLOCK_ADDITION_IDS: &[u32] = &[EBML_ID_BLOCK_MORE];
        while !state.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, BLOCK_ADDITION_IDS)?
            else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            if header.id == EBML_ID_BLOCK_MORE
                && let Some(child_depth) = next_mkv_deep_probe_depth(depth)
            {
                self.scan_block_more_for_hdr10plus(child_end, state, child_depth)?;
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    pub(super) fn scan_block_more_for_hdr10plus(
        &mut self,
        end: u64,
        state: &mut Hdr10PlusProbeState,
        _depth: usize,
    ) -> Result<(), MediaInfoError> {
        const BLOCK_MORE_IDS: &[u32] = &[EBML_ID_BLOCK_ADDITIONAL];
        while !state.done() && self.position()? < end {
            let Some(header) = self.read_deep_probe_candidate_header(end, BLOCK_MORE_IDS)? else {
                break;
            };
            let child_end = header.end(end, self.file_len);
            if header.id == EBML_ID_BLOCK_ADDITIONAL {
                let remaining = state.limits.max_bytes_read.saturating_sub(state.bytes_read);
                let to_read = child_end
                    .saturating_sub(header.data_offset)
                    .min(state.limits.block_additional_peek_bytes)
                    .min(remaining);
                if to_read > 0 {
                    self.seek_to(header.data_offset)?;
                    let payload = self.read_bytes(to_read)?;
                    state.bytes_read += to_read;
                    if scan_itu_t35_payload_for_hdr10plus(&payload) {
                        state.found = true;
                    }
                }
            }
            self.seek_to(child_end)?;
        }
        Ok(())
    }

    pub(super) fn inspect_video_block_for_hdr10plus(
        &mut self,
        block: &BlockHeaderInfo,
        state: &mut Hdr10PlusProbeState,
    ) -> Result<(), MediaInfoError> {
        if state.done() || block.lacing_type != 0 || block.payload_size == 0 {
            return Ok(());
        }
        let remaining = state.limits.max_bytes_read.saturating_sub(state.bytes_read);
        let to_read = block.payload_size.min(remaining);
        if to_read == 0 {
            return Ok(());
        }
        self.seek_to(block.payload_offset)?;
        let payload = self.read_bytes(to_read)?;
        state.bytes_read += to_read;
        state.inspected_blocks += 1;
        if scan_hevc_frame_for_hdr10plus(&payload, state.nal_length_size) {
            state.found = true;
        }
        Ok(())
    }

    pub(super) fn inspect_audio_block_for_profile(
        &mut self,
        block: &BlockHeaderInfo,
        codec_name: &str,
        header_strip_prefix: Option<&[u8]>,
        state: &mut AudioProfileProbeState,
    ) -> Result<(), MediaInfoError> {
        if state.done(codec_name) || block.payload_size == 0 {
            return Ok(());
        }

        let probe_spec = audio_profile_probe_spec(Some(codec_name));
        let prefix_probe_bytes = audio_header_probe_bytes(codec_name).max(probe_spec.prefix_bytes);
        if prefix_probe_bytes == 0 {
            return Ok(());
        }

        self.seek_to(block.payload_offset)?;
        let Some((frame_header_size, frame_size)) =
            parse_first_laced_frame_from_reader(self, block.payload_size, block.lacing_type)?
        else {
            return Ok(());
        };
        let frame_offset = block.payload_offset.saturating_add(frame_header_size);
        let probe_payload_size = frame_size;
        let remaining = MKV_AUDIO_PROFILE_SCAN_MAX_BYTES.saturating_sub(state.bytes_read);
        let header_strip_len = header_strip_prefix.map_or(0, <[u8]>::len);
        let file_prefix_size = probe_payload_size
            .min(prefix_probe_bytes.saturating_sub(header_strip_len) as u64)
            .min(remaining);
        if file_prefix_size == 0 && header_strip_len == 0 {
            return Ok(());
        }

        self.seek_to(frame_offset)?;
        let mut prefix =
            Vec::with_capacity(header_strip_len.saturating_add(file_prefix_size as usize));
        if let Some(header_strip_prefix) = header_strip_prefix {
            prefix.extend_from_slice(header_strip_prefix);
        }
        if file_prefix_size > 0 {
            prefix.extend_from_slice(&self.read_bytes(file_prefix_size)?);
        }
        state.bytes_read += file_prefix_size;

        let suffix = if probe_spec.suffix_bytes > 0 {
            let remaining = MKV_AUDIO_PROFILE_SCAN_MAX_BYTES.saturating_sub(state.bytes_read);
            let suffix_size = probe_payload_size
                .min(probe_spec.suffix_bytes as u64)
                .min(remaining);
            let suffix_offset =
                frame_offset.saturating_add(probe_payload_size.saturating_sub(suffix_size));
            if suffix_size > 0 && suffix_offset >= frame_offset + file_prefix_size {
                self.seek_to(suffix_offset)?;
                let suffix = self.read_bytes(suffix_size)?;
                state.bytes_read += suffix_size;
                Some(suffix)
            } else {
                None
            }
        } else {
            None
        };

        state.inspected_blocks += 1;
        state.merge_profile(
            codec_name,
            detect_audio_profile_from_probe_bytes(Some(codec_name), &prefix, suffix.as_deref()),
        );
        let parsed_channels = audio_channels_from_probe_bytes(codec_name, &prefix);
        if matches!(codec_name, "mp1" | "mp2" | "mp3") && state.mpeg_header.is_none() {
            state.mpeg_header = crate::codec::mpeg_audio::find_mpeg_audio_header(&prefix);
        }
        let dolby_header = match codec_name {
            "ac3" => find_ac3_header(&prefix),
            "eac3" => find_eac3_header(&prefix),
            _ => None,
        };
        if let Some(header) = dolby_header
            && state
                .dolby_header
                .is_none_or(|previous| header.channels > previous.channels)
        {
            state.dolby_header = Some(header);
        }
        if parsed_channels.is_some_and(|channels| channels > state.channels.unwrap_or(0)) {
            state.channels = parsed_channels;
        }
        Ok(())
    }

    pub(super) fn read_deep_probe_candidate_header(
        &mut self,
        end: u64,
        candidate_ids: &[u32],
    ) -> Result<Option<EbmlElementHeader>, MediaInfoError> {
        let start = self.position()?;
        if start < end {
            // Most callers just sought to an element boundary. Avoid reading
            // a search window into its media payload when the header matches.
            if let Ok(Some(header)) = self.read_element_header()
                && candidate_ids.contains(&header.id)
                && header.data_offset <= header.end(end, self.file_len)
            {
                return Ok(Some(header));
            }
            self.seek_to(start)?;
        }
        while self.position()? < end {
            let Some(candidate_pos) = self.find_next_deep_probe_candidate(end, candidate_ids)?
            else {
                return Ok(None);
            };
            self.seek_to(candidate_pos)?;
            let Some(header) = self.read_element_header()? else {
                return Ok(None);
            };
            let child_end = header.end(end, self.file_len);
            if candidate_ids.contains(&header.id)
                && header.data_offset <= child_end
                && child_end <= end.min(self.file_len)
            {
                return Ok(Some(header));
            }
            self.seek_to(candidate_pos.saturating_add(1))?;
        }

        Ok(None)
    }

    pub(super) fn find_next_deep_probe_candidate(
        &mut self,
        end: u64,
        candidate_ids: &[u32],
    ) -> Result<Option<u64>, MediaInfoError> {
        let mut pos = self.position()?;
        let mut buf = vec![0_u8; MKV_DEEP_PROBE_CANDIDATE_READ_BYTES];
        while pos < end {
            let read_len = usize::try_from(
                end.saturating_sub(pos)
                    .min(MKV_DEEP_PROBE_CANDIDATE_READ_BYTES as u64),
            )
            .unwrap_or(MKV_DEEP_PROBE_CANDIDATE_READ_BYTES);
            if read_len == 0 {
                return Ok(None);
            }
            self.seek_to(pos)?;
            let mut bytes_read = 0;
            while bytes_read < read_len {
                let read = self
                    .read(&mut buf[bytes_read..read_len])
                    .map_err(|e| MediaInfoError::Io(e.to_string()))?;
                if read == 0 {
                    break;
                }
                bytes_read += read;
            }
            if bytes_read == 0 {
                return Ok(None);
            }
            if let Some(candidate) = scan::find_ebml_candidate(&buf[..bytes_read], 0, candidate_ids)
            {
                return Ok(Some(pos + candidate.offset as u64));
            }
            if bytes_read < read_len || read_len < buf.len() {
                return Ok(None);
            }
            pos = pos.saturating_add((read_len.saturating_sub(3)) as u64);
        }

        Ok(None)
    }

    pub(super) fn read_element_header(
        &mut self,
    ) -> Result<Option<EbmlElementHeader>, MediaInfoError> {
        let Some((id, _)) = read_ebml_id_from_reader(self)? else {
            return Ok(None);
        };
        let (size, _) = read_ebml_size_from_reader(self)?;
        let data_offset = self.position()?;
        Ok(Some(EbmlElementHeader {
            id,
            data_offset,
            size,
        }))
    }

    pub(super) fn read_sized_payload(
        &mut self,
        header: EbmlElementHeader,
        parent_end: u64,
    ) -> Result<Option<Vec<u8>>, MediaInfoError> {
        let child_end = header.end(parent_end, self.file_len);
        let Some(size) = header.size else {
            self.seek_to(child_end)?;
            return Ok(None);
        };
        let declared_end = header
            .data_offset
            .checked_add(size)
            .ok_or_else(|| MediaInfoError::Parse("MKV element size overflow".into()))?;
        if declared_end > parent_end || declared_end > self.file_len {
            return Err(MediaInfoError::Parse(format!(
                "MKV element 0x{:X} declares {} bytes beyond remaining input",
                header.id, size
            )));
        }
        if size > MKV_KEEP_ELEMENT_MAX_BYTES {
            return Err(MediaInfoError::Parse(format!(
                "MKV element 0x{:X} exceeds parser budget",
                header.id
            )));
        }
        self.metadata_payload_bytes = self
            .metadata_payload_bytes
            .checked_add(size)
            .ok_or_else(|| MediaInfoError::Parse("MKV metadata budget overflow".into()))?;
        if self.metadata_payload_bytes > MKV_METADATA_AGGREGATE_MAX_BYTES {
            return Err(MediaInfoError::Parse(
                "MKV metadata output exceeds parser budget".into(),
            ));
        }
        self.seek_to(header.data_offset)?;
        let payload = self.read_bytes(size)?;
        self.seek_to(child_end)?;
        Ok(Some(payload))
    }

    pub(super) fn read_top_level_payload_at(
        &mut self,
        offset: u64,
        expected_id: u32,
    ) -> Result<Option<Vec<u8>>, MediaInfoError> {
        self.seek_to(offset)?;
        let Some(header) = self.read_element_header()? else {
            return Ok(None);
        };
        if header.id != expected_id {
            return Ok(None);
        }
        self.read_sized_payload(header, self.file_len)
    }

    pub(super) fn read_block_header(
        &mut self,
        data_size: u64,
        cluster_timestamp: u64,
    ) -> Result<Option<BlockHeaderInfo>, MediaInfoError> {
        if data_size < 4 {
            return Ok(None);
        }

        let start = self.position()?;
        let peek_len = data_size.min(11) as usize;
        let mut buf = [0_u8; 11];
        self.read_exact(&mut buf[..peek_len])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;

        let Some((track_number, track_len)) = parse_ebml_vint_value(&buf[..peek_len]) else {
            return Ok(None);
        };
        if data_size < track_len as u64 + 3 || peek_len < track_len + 3 {
            return Ok(None);
        }

        let relative_timestamp = i16::from_be_bytes([buf[track_len], buf[track_len + 1]]) as i64;
        let Some(timestamp) = cluster_timestamp.checked_add_signed(relative_timestamp) else {
            return Ok(None);
        };

        let flags = buf[track_len + 2];
        let payload_offset = start + track_len as u64 + 3;
        let payload_size = data_size - track_len as u64 - 3;
        self.seek_to(payload_offset)?;

        Ok(Some(BlockHeaderInfo {
            track_number,
            timestamp,
            payload_offset,
            payload_size,
            lacing_type: (flags & 0x06) >> 1,
            invisible: flags & 0x08 != 0,
        }))
    }

    pub(super) fn read_unsigned_payload(
        &mut self,
        size: u64,
    ) -> Result<Option<u64>, MediaInfoError> {
        if size == 0 || size > 8 {
            return Ok(None);
        }
        let mut buf = [0_u8; 8];
        let size = size as usize;
        self.read_exact(&mut buf[..size])
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        Ok(parse_ebml_uint(&buf[..size]))
    }

    pub(super) fn read_bytes(&mut self, size: u64) -> Result<Vec<u8>, MediaInfoError> {
        let mut buf = vec![0_u8; size as usize];
        self.read_exact(&mut buf)
            .map_err(|e| MediaInfoError::Io(e.to_string()))?;
        Ok(buf)
    }

    pub(super) fn position(&mut self) -> Result<u64, MediaInfoError> {
        Ok(self.pos)
    }

    pub(super) fn seek_to(&mut self, pos: u64) -> Result<(), MediaInfoError> {
        if self.pos == pos {
            return Ok(());
        }
        self.reader
            .seek(SeekFrom::Start(pos))
            .map(|new_pos| {
                self.pos = new_pos;
            })
            .map_err(|e| MediaInfoError::Io(e.to_string()))
    }
}

impl<R: Read> Read for MkvRawScanner<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bytes_read = self.reader.read(buf)?;
        self.pos = self.pos.saturating_add(bytes_read as u64);
        Ok(bytes_read)
    }
}

pub(super) fn probe_mkv_deep_metadata<R: Read + Seek>(
    scanner: &mut MkvRawScanner<R>,
    frame_rate: Option<MkvFrameRateProbeRequest>,
    hdr10plus: Option<MkvHdr10PlusProbeRequest>,
    audio: Vec<MkvAudioProbeRequest>,
) -> MkvDeepProbeResult {
    let mut plan = MkvDeepProbePlan::new(frame_rate, hdr10plus, audio);
    if plan.is_empty() {
        return MkvDeepProbeResult::default();
    }

    if scanner.run_deep_probe_plan(&mut plan).is_err() {
        return MkvDeepProbeResult::default();
    }
    plan.into_result()
}

pub(super) fn audio_header_probe_bytes(codec_name: &str) -> usize {
    match codec_name {
        "ac3" => 7,
        "eac3" => 6,
        "dts" => 32,
        "mp1" | "mp2" | "mp3" => 4,
        _ => 0,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct MkvFrameTiming {
    pub(super) rate: scryer_media_model::Rational,
    pub(super) variable: bool,
}

pub(super) fn observe_frame_timing(
    timestamps: &[u64],
    timestamp_scale_ns: f64,
) -> Option<MkvFrameTiming> {
    if timestamps.len() < 4
        || !timestamp_scale_ns.is_finite()
        || timestamp_scale_ns < 1.0
        || timestamp_scale_ns.fract() != 0.0
        || timestamp_scale_ns >= u64::MAX as f64
    {
        return None;
    }
    // Blocks arrive in coding order; timing describes presentation order.
    let mut ordered = timestamps.to_vec();
    ordered.sort_unstable();
    let deltas: Vec<u64> = ordered.windows(2).map(|pair| pair[1] - pair[0]).collect();
    if deltas.contains(&0) {
        return None;
    }
    let span = (ordered.last()? - ordered.first()?).checked_mul(timestamp_scale_ns as u64)?;
    let numerator = i64::try_from(deltas.len())
        .ok()?
        .checked_mul(1_000_000_000)?;
    let rate = scryer_media_model::Rational::new(numerator, span)?;
    if !is_plausible_frame_rate(Some(rate.numerator as f64 / rate.denominator as f64)) {
        return None;
    }
    // One tick of alternating quantization is not evidence of VFR.
    let variable = deltas.iter().max()? - deltas.iter().min()? > 1;
    if variable && timestamps.windows(2).any(|pair| pair[1] < pair[0]) {
        // A bounded coding-order sample can end before reordered frames arrive.
        // Its gaps cannot establish a variable presentation cadence.
        return None;
    }
    Some(MkvFrameTiming { rate, variable })
}

#[cfg(test)]
pub(super) fn estimate_frame_rate_from_timestamps(
    timestamps: &[u64],
    timestamp_scale_ns: f64,
) -> Option<f64> {
    let timing = observe_frame_timing(timestamps, timestamp_scale_ns)?;
    Some(timing.rate.numerator as f64 / timing.rate.denominator as f64)
}

pub(super) fn parse_first_laced_frame_from_reader<R: Read>(
    reader: &mut R,
    payload_size: u64,
    lacing_type: u8,
) -> Result<Option<(u64, u64)>, MediaInfoError> {
    if payload_size == 0 {
        return Ok(None);
    }
    if lacing_type == 0 {
        return Ok(Some((0, payload_size)));
    }
    if payload_size < 1 {
        return Ok(None);
    }

    let lace_count = u64::from(read_u8_from_reader(reader)?) + 1;
    let mut header_size = 1_u64;
    let mut remaining_payload = payload_size.saturating_sub(1);

    if lace_count <= 1 {
        return Ok(Some((header_size, remaining_payload)));
    }

    match lacing_type {
        1 => {
            let mut total_sizes = 0_u64;
            let mut first_frame_size = None;

            for index in 0..lace_count - 1 {
                let mut size = 0_u64;
                loop {
                    if remaining_payload == 0 {
                        return Ok(None);
                    }
                    let byte = u64::from(read_u8_from_reader(reader)?);
                    header_size += 1;
                    remaining_payload -= 1;
                    let Some(next_size) = size.checked_add(byte) else {
                        return Ok(None);
                    };
                    size = next_size;
                    let Some(next_total) = total_sizes.checked_add(byte) else {
                        return Ok(None);
                    };
                    total_sizes = next_total;
                    if byte != 0xFF {
                        break;
                    }
                }
                if index == 0 {
                    first_frame_size = Some(size);
                }
            }

            if remaining_payload < total_sizes {
                return Ok(None);
            }

            Ok(Some((
                header_size,
                first_frame_size.unwrap_or(remaining_payload),
            )))
        }
        2 => {
            if !remaining_payload.is_multiple_of(lace_count) {
                return Ok(None);
            }
            Ok(Some((header_size, remaining_payload / lace_count)))
        }
        3 => {
            let (first_frame_size, first_size_len) = read_ebml_vint_value_from_reader(reader)?;
            header_size += first_size_len as u64;
            let Some(next_remaining) = remaining_payload.checked_sub(first_size_len as u64) else {
                return Ok(None);
            };
            remaining_payload = next_remaining;

            let mut total_sizes = first_frame_size;
            let Ok(mut previous_size) = i64::try_from(first_frame_size) else {
                return Ok(None);
            };

            for _ in 1..lace_count - 1 {
                let (delta, delta_len) = read_ebml_sint_value_from_reader(reader)?;
                header_size += delta_len as u64;
                let Some(next_remaining) = remaining_payload.checked_sub(delta_len as u64) else {
                    return Ok(None);
                };
                remaining_payload = next_remaining;

                let Some(next_size) = previous_size.checked_add(delta) else {
                    return Ok(None);
                };
                previous_size = next_size;
                if previous_size < 0 {
                    return Ok(None);
                }
                let Some(next_total) = total_sizes.checked_add(previous_size as u64) else {
                    return Ok(None);
                };
                total_sizes = next_total;
            }

            if remaining_payload < total_sizes {
                return Ok(None);
            }

            Ok(Some((header_size, first_frame_size)))
        }
        _ => Ok(None),
    }
}

pub(super) fn is_plausible_frame_rate(frame_rate_fps: Option<f64>) -> bool {
    frame_rate_fps.is_some_and(|fps| (1.0..=240.0).contains(&fps))
}

pub(super) fn should_replace_frame_rate(existing: Option<f64>, observed: f64) -> bool {
    match existing {
        None => true,
        Some(current) if current <= 0.0 => true,
        Some(current) => current < 10.0 && observed >= current * 10.0,
    }
}
