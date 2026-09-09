# Native analysis fixtures

Native expected-value tests run without FFprobe. `native_tests.rs` checks the authored container corpus, video sequence/color metadata, encoded audio properties, and chapter inventories. Synthetic filesystem, playlist, navigation, timing, malformed-input, and bounded-read fixtures live beside their implementations. `support/disc_image.rs` also supplies authored images to application import tests.

The AV1 and HEVC sequence fixtures contain one second of black 64×64 video at 24 fps, with 10-bit 4:2:0, BT.2020 primaries/matrix, and PQ transfer. The AV1 fixture was encoded with SVT-AV1 4.1.0. HEVC also signals mastering luminance 1000/0.005 cd/m² and content light levels 1000/400. Each MKV is a remux of its MP4 video stream.

`chapters_nero.mp4` and `chapters_quicktime.mp4` remux the HEVC sequence fixture with two authored chapters: “Opening” at 0–0.5 seconds and “幕間” at 0.5–1 second. FFmpeg 8.1.1 created both; `-movflags +faststart+disable_chpl` omits the Nero list in the QuickTime fixture. Neither file contains subtitle dialogue. Native tests check titles, precise start/end times, and exclusion of chapter text from subtitle scoring.

`ps_mpeg1_mp2.mpg` and `ps_mpeg2_mp2.vob` contain two seconds of 160×120 `testsrc2` at 25 fps and a 440 Hz mono tone sampled at 48 kHz. FFmpeg 8.1.1 generated them with video targets of 200 kbps and MP2 audio at 128 kbps, using the `mpeg` and `vob` muxers respectively. Native expectations require the correct MPEG video family, 8-bit 4:2:0, square pixels, rational timing, MPEG-2 Main profile, and mono MP2 metadata. Sequence-header bitrate sentinels remain unknown. These are ordinary program-stream fixtures, not authored DVD navigation.

The application regression `canonical_catalog_and_import_paths_preserve_the_same_analysis_contract` runs 42 fixtures through `NativeMediaAnalyzer`, the actual post-download import gate, and the rule projection. It compares the complete analysis contract, excluding elapsed probe time. Coverage includes MPEG-PS, ASF, FLV, AC-3/E-AC-3, Opus, Vorbis, FLAC, and AVI stream sampling, alongside video sequence metadata, MP4 chapters, and the four sparse Matroska fixtures described below. The single-sample HDR10+ MP4 retains its explicit sample-table frame-rate declaration without asserting an observed cadence.

Matroska timing regressions retain declarations separately from rational observations and sampled VFR status. They cover presentation reordering, millisecond rounding, duplicate timestamps, invisible frames, lacing, and nondefault track scales. Laced or inconclusive timing remains unknown. An instrumented source requires the timing probe to stop after 12 timestamps and read fewer than 4 KiB from a fixture with 48 large blocks; this does not measure every other enrichment pass.

Concurrent production-path regressions hold one native reader open until both the catalog scan and final import gate join it. They require identical projected metadata, reject results when the source changes during the shared probe, and route changed sources to review for both automatic and operator-queued imports. Separate coordinator tests cover cancellation, worker failure, retries, source versions, and distinct disc selections.

ASF header regressions retain accessible sibling streams and timing when nested extensions exceed the eight-level inspection limit. The report must explicitly mark incomplete metadata and an exhausted budget; the aggregate header byte and object-count limits still apply.

The supplemental fixture SHA-256 values are:

```text
c8c2409bf706fc49e0b6d4e5ff487ca6a97c3e2bd3a35ebccb0819756cd76602  av1_sequence_pq.mp4
43880e77806201ec7718734fed3acfb62db5489831cffa76e6702eb565ef33c8  av1_sequence_pq.mkv
bfbb700dd364d80a840b36f88a7ce56fee0fe062014132ac9cd8b73b15f579c9  hevc_sequence_pq.mp4
00f9a98d74fe4784618a5abd3f544d497e299a7c297d28d9e0323b8e7ee94e98  hevc_sequence_pq.mkv
874da880e21134b5ee191c7a1bc52cf2c247e61c711ede03c88e7fbb32b80bf6  chapters_nero.mp4
1344fa8f6c1fec10eda5720f2e8c4da157665c96ab0a097ff4f97daccf9a5c00  chapters_quicktime.mp4
a9faea43ed2067d7debf9ee423fa5577e81ad5f2050b6f3e3e4ba3387bb2ff13  ps_mpeg1_mp2.mpg
cc05b9fc1ccea97944a2a15e46515e65b82140dd6444ebfbc807bbf3b42df312  ps_mpeg2_mp2.vob
```

## Differential checks

Run the opt-in reference check explicitly:

```sh
cargo nextest run -p scryer-mediainfo --run-ignored all \
  -E 'test(compare_fixture_corpus_against_ffprobe)' --success-output immediate
```

`SCRYER_PARITY_FIXTURES` optionally selects comma-separated fixture basenames. Empty or unset selects the entire corpus. A requested missing file fails the check. `SCRYER_PARITY_REPORT` optionally writes a JSON report containing the analysis revision, full FFprobe version, native/reference metadata, and every comparison mismatch. Missing required native values count as mismatches. The reference requests chapters and programs explicitly and samples up to 16 PQ video frames for side metadata.

The `media-probe-parity` workflow is manually dispatched. It runs native expectations and production-path consistency before the reference comparison, and retains version, source revision, logs, and structured comparison evidence. The reference tools are development-only; production probing has no FFmpeg dependency.

FFprobe's `sample_fmt` describes a decoder's chosen output representation. It is retained in reference reports but does not define the encoded representation of compressed audio. Native fixtures separately require PCM representation and explicit container sample bit depth; compressed decoder output formats stay unknown.

E-AC-3 frame-size/block-duration bitrate in transport streams and Matroska is compared as an estimate unless explicit stream statistics are present. The native estimate remains separate from measured bitrate. MP4 sample-table accounting and Matroska BPS statistics still require measured bitrate and its provenance; omitting both measured and estimated values remains a mismatch.

FFprobe 8.1.1 reports MPEG-1's unspecified/VBR sequence bitrate code (`0x3ffff`) as 104,857,200 bps. The comparison retains that raw reference output but excludes that specific codec/value pair from required bitrate measurements. Neighboring valid declarations remain mandatory, and native fixtures require the unspecified bitrate to stay unknown.

FFprobe's [ASF demuxer](https://github.com/FFmpeg/FFmpeg/blob/master/libavformat/asfdec_f.c) converts stored RFC 1766 language tags to ISO 639 codes and discards region information. For ASF only, the comparison accepts a known two-letter primary tag when its ISO 639-3 equivalent matches the reference. Missing or conflicting originals remain mismatches. Native fixtures independently require exact originals such as `en` and `ja-JP` alongside normalized values.

Frame-rate comparison prefers a valid native declaration over a bounded observation and uses the reference's average rate, falling back to its base rate only when the average is unavailable. The base rate can be a multiple of the actual frame cadence. A short, quantized observation does not replace a valid declaration in this comparison; native timing fixtures independently verify sampled rationals and VFR uncertainty. Missing rates and contradictory declarations remain mismatches.

For AVI fixtures, the reference runner also requests video packet sizes, timestamps, and durations, with a 4,097-packet ceiling. Accounting is accepted only when at most 4,096 packets exactly match the declared stream frame count and form a contiguous timeline. The final packet's duration is included. Raw FFprobe bitrate values and packet headers remain in the report beside this independent accounting; missing native measured values still fail. This avoids comparing complete native accounting against a reference estimate that uses only the first-to-last timestamp span. No packet payloads are requested.

The four original `dv_profile5.mkv`, `dv_profile7.mkv`, `dv_profile8.mkv`, and `hevc_hdr10plus.mkv` files are 283–294-byte metadata stubs. They have no language elements, no audio CodecPrivate or audio blocks, no DefaultDuration, and only two placeholder video blocks. `support/sparse_mkv.rs` reconstructs their complete authored bytes independently of the parser; a byte change invalidates their reference expectations. Mandatory native checks require language, channel layout, and cadence to remain unknown.

For those exact stubs, FFprobe 8.1.1's [Matroska demuxer](https://github.com/FFmpeg/FFmpeg/blob/n8.1.1/libavformat/matroskadec.c) supplies default English tags and synthesizes AAC configuration from channel count. Its [stream analysis](https://github.com/FFmpeg/FFmpeg/blob/n8.1.1/libavformat/demux.c) falls back to inverse time base for the video rate. The report retains the raw `reference`, a separate `comparison_reference`, and each `reference_assumptions` entry with its reason. Only the audited default values are removed from the comparison view; unfamiliar reference values or changed stream inventories fail. No general missing-value exemption applies to other files.

Explicit variants of all four stubs add stored English tags, a 25 fps DefaultDuration, and AAC-LC stereo AudioSpecificConfig. Native expectations require those facts. An opt-in differential test compares the variants directly with FFprobe and deliberately removes each native fact to prove the comparison fails. The reference CI job runs both the negative and positive checks. These variants remain metadata fixtures, not independently playable video samples.

The differential harness is deliberately strict and is also useful for tracking remaining gaps. A successful subset run does not establish complete-corpus parity or complete-file integrity.

## Disc fixture coverage

The shared ISO9660 and UDF builders generate small synthetic navigation files around existing elementary-stream fixtures. UDF cases cover physical and metadata partitions, split metadata extents, large sparse files, descriptor continuations, prevailing descriptor revisions, backup anchors, and unsupported partition maps. They are parser regressions, not independently certified disc-authoring output.

Blu-ray cases require CLPI clock ranges and use CPI access-point maps for trimmed payload inspection. Probe ranges include surrounding access units; exact playlist in/out times determine runtime. The same physical clip can supply distinct packet ranges without sharing a failed or incompatible cached result. A regression places scrambled and clear portions in one clip and checks each cut independently through both ISO9660 and UDF. Missing CLPI metadata remains incomplete and requires import review.

Application fixtures exercise automatic review holds, intact manual imports, persisted title selection, and explicit episode mappings against one physical image. Unsupported navigation, encryption, and incomplete inspection remain separate from proven malformed structures.
