# Rule input fields

A rule reads only these paths. The validator rejects any other `input.` path
(`RULE_UNKNOWN_INPUT_PATH`) and any path it cannot resolve statically, such as
`input.release[x]` (`RULE_DYNAMIC_INPUT_PATH`). `?` means the value can be
null. Language codes are ISO 639-3 (`eng`, `jpn`, `fra`).

## input.release: the release being scored

| Field | Type | Notes |
|---|---|---|
| `raw_title` | string | Unparsed release name |
| `protocol` | string? | `usenet`, `torrent`, `http` |
| `quality` | string? | Tier: `2160P`, `1080P`, `720P`, ... |
| `source` | string? | `WEB-DL`, `WEBRip`, `BLURAY`, `HDTV`, ... |
| `video_codec` | string? | `H.265`, `H.264`, `AV1`, ... |
| `audio` | string? | Primary audio codec (`DDP`, `AAC`, `TrueHD`) |
| `audio_codecs` | string[] | All detected audio codecs |
| `audio_channels` | string? | `5.1`, `7.1`, `2.0` |
| `languages_audio` | string[] | From name and indexer listing |
| `indexer_languages` | string[] | Only what the indexer reported |
| `languages_subtitles` | string[] | |
| `is_dual_audio`, `is_atmos`, `is_dolby_vision`, `has_hdr_fallback`, `detected_hdr`, `is_hdr10plus`, `is_hlg`, `is_10bit` | bool | Name-derived video and audio flags |
| `is_remux`, `is_bd_disk` | bool | Full remux; Blu-ray disc |
| `is_proper_upload`, `is_repack` | bool | Tags |
| `revision` | int | 0 original, 1 PROPER/REPACK/RERIP, 2 REPACK2, ... |
| `is_ai_enhanced`, `is_hardcoded_subs`, `is_uncensored`, `is_dubs_only`, `is_obfuscated`, `is_retagged` | bool | Name markers |
| `is_password_protected` | bool? | Indexer flag; null when unknown |
| `has_release_group` | bool | A non-empty group was parsed |
| `release_group` | string? | |
| `streaming_service` | string? | Service tag when present |
| `edition` | string? | `extended`, `IMAX`, `criterion`, ... |
| `anime_version` | number? | v2, v3 ... |
| `episode_release_type` | string? | `single_episode`, `multi_episode`, `season_pack`, `unknown` |
| `is_season_pack`, `is_multi_episode` | bool | |
| `year` | number? | |
| `parse_confidence` | float | 0.0 to 1.0 |
| `size_bytes` | number? | Use `scryer.size_gib()`; may be null |
| `age_days` | number? | Days since the indexer published it |
| `thumbs_up`, `thumbs_down` | number? | Indexer votes |
| `normalized_tokens` | string[] | Ordered parser tokens |
| `stereoscopy_tokens` | string[] | `stereo:three_d`, `stereo:layout:<layout>`, ... |
| `stereoscopy` | object? | `presentation`, `layout?`, `sampling?`, `encoding?` |
| `extra` | object | Indexer attributes as `input.release.extra.<key>`. Can be empty, so a missing key is undefined, not null. |

## input.profile: the quality profile in force

`id`, `name`, `quality_tiers[]`, `archival_quality?`, `allow_unknown_quality`,
`source_allowlist[]`, `source_blocklist[]`, `video_codec_allowlist[]`,
`video_codec_blocklist[]`, `audio_codec_allowlist[]`, `audio_codec_blocklist[]`,
`atmos_preferred`, `dolby_vision_allowed`, `detected_hdr_allowed`,
`prefer_remux`, `allow_bd_disk`, `allow_upgrades`, `prefer_dual_audio`,
`required_audio_languages[]`, `scoring_persona` (`balanced`, `audiophile`,
`efficient`, `compatible`).

`scoring_overrides`: `allow_x265_non4k?`, `block_dv_without_fallback?`,
`prefer_compact_encodes?`, `prefer_lossless_audio?`, `block_upscaled?`. Null
means the persona default applies.

`input.profile.id` lets one rule set score differently per profile.

## input.context: the title being searched for

| Field | Type | Notes |
|---|---|---|
| `media_type` | string | `movie`, `series`, `anime` |
| `category` | string | Category hint |
| `title_id` | string? | Scryer title id |
| `library_name` | string? | |
| `original_language`, `original_country` | string? | From metadata |
| `inferred_original_audio_language` | string | Assumed for unlabeled releases |
| `tags` | string[] | Title tags, lowercase |
| `has_existing_file` | bool | |
| `existing_score` | number? | Built-in score of the current file |
| `search_mode` | string | `auto` or `post_download` |
| `runtime_minutes` | number? | |
| `coverage_total_runtime_minutes`, `coverage_member_runtime_minutes`, `coverage_member_count` | number? | Pack coverage |
| `is_anime`, `is_filler` | bool | |

## input.builtin_score: Scryer's own score before user rules

`total` (number), `blocked` (bool), `codes` (string[], e.g. `quality_tier_0`).

## input.file: the downloaded file, after media analysis

`input.file` is **null before download**. Guard every use with
`input.file != null`, or the rule simply never matches during search.

`video_codec?`, `video_width?`, `video_height?`, `video_bitrate_kbps?`,
`video_bit_depth?`, `video_hdr_format?`, `dovi_profile?`, `dovi_bl_compat_id?`,
`video_frame_rate?`, `video_profile?`, `audio_codec?`, `audio_profile?`,
`audio_channels?`, `audio_bitrate_kbps?`, `audio_languages[]`,
`subtitle_languages[]`, `subtitle_codecs[]`, `has_multiaudio`,
`duration_seconds?`, `num_chapters?`, `container_format?`.

`audio_streams[]`: `codec?`, `profile?`, `channels?`, `language?`, `bitrate_kbps?`.
`subtitle_streams[]`: `codec?`, `language?`, `name?`, `forced`, `default`.

## Builtins

| Function | Returns |
|---|---|
| `scryer.block_score()` | -10000 |
| `scryer.size_gib(bytes)` | GiB as a float (0 for a non-number) |
| `scryer.lang_matches(code, pattern)` | bool. Matches ISO 639 codes including common aliases. |
| `scryer.normalize_source(raw)` | normalized source string |
| `scryer.normalize_codec(raw)` | normalized codec string |

Standard Rego builtins work, including `regex.match`, `contains`, `lower`,
`startswith`, `count` and `object.get`. Network, random and runtime builtins
are not built in. `time.now_ns()` works but makes results time dependent,
so avoid it.
