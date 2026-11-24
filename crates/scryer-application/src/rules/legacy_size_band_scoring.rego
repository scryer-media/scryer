package scryer.rules.user.template

import rego.v1

# Size classifications are explanatory only. The host owns codec-aware
# eligibility bounds and size-fit search ranking; size adds no intrinsic points.
native_size_persona := lower(object.get(input.profile, "scoring_persona", "balanced"))
native_size_category := "anime" if { lower(object.get(input.context, "category", "")) == "anime" } else := "series" if { lower(object.get(input.context, "category", "")) == "series" } else := "movie"

native_size_movie_2160_bitrate := 32 if { native_size_persona == "balanced" } else := 57
native_size_quality := upper(value) if { value := object.get(input.release, "quality", null); value != null } else := "UNKNOWN"
native_size_bitrate := value if {
  table := {
    "movie": {"4320P": 142.5, "2160P": native_size_movie_2160_bitrate, "1080P": 9.1, "720P": 3.4, "576P": 2.4, "480P": 1.4, "unknown": 6.8},
    "series": {"4320P": 55, "2160P": 22, "1080P": 8.5, "720P": 3.3, "576P": 2.4, "480P": 1.4, "unknown": 5.5},
    "anime": {"4320P": 70, "2160P": 28, "1080P": 8.5, "720P": 3.4, "576P": 2.4, "480P": 1.4, "unknown": 5.7},
  }[native_size_category]
  value := object.get(table, native_size_quality, table["unknown"])
}
native_size_codec_factor := 0.5 if { input.release.video_codec in {"AV1", "VVC"} } else := 0.75 if { input.release.video_codec in {"H.265", "VP9"} } else := 1.1 if { input.release.video_codec == "H.264" } else := 1.3 if { input.release.video_codec == "VC1" } else := 1.8 if { input.release.video_codec == "MPEG2" } else := 1.5 if { input.release.video_codec in {"MPEG4", "XVID", "DIVX"} } else := 1
native_size_remux_factor := 1.45 if { input.profile.prefer_remux == true } else := 1 if { native_size_persona == "balanced" } else := 1.45
native_size_bluray_factor := 1.35 if { input.release.source in {"BluRay", "BR-DISK"} } else := 1
native_size_remux_source_factor := native_size_remux_factor if {
  input.release.is_remux == true
  native_size_category != "anime"
} else := 1
native_size_disk_factor := 1.8 if { input.release.is_bd_disk == true } else := 1
native_size_web_factor := 0.8 if { input.release.source in {"WEB-DL", "WEBRip"} } else := 1
native_size_source_factor := native_size_bluray_factor * native_size_remux_source_factor * native_size_disk_factor * native_size_web_factor
native_size_default_runtime := 120 if { native_size_category == "movie" } else := 45 if { native_size_category == "series" } else := 24
native_size_runtime(field) := value if { candidate := object.get(input.context, field, null); candidate != null; candidate > 0; value := candidate } else := native_size_default_runtime
native_size_expected_gib(field) := max([native_size_bitrate * native_size_codec_factor * native_size_source_factor * (native_size_runtime(field) * 60) / 8 / 1024, 0.5])
native_size_total_ratio := input.release.size_bytes / 1073741824 / native_size_expected_gib("coverage_total_runtime_minutes")
native_size_member_ratio := input.release.size_bytes / 1073741824 / native_size_expected_gib("coverage_member_runtime_minutes")
native_size_member_count := value if { value := object.get(input.context, "coverage_member_count", null); value != null } else := 1

native_size_thresholds := [8, 4, 2.4, 1.8, 1.35, 1, 0.75, 0.55, 0.35, 0.1] if { native_size_category == "movie" } else := [8, 4, 2.4, 1.8, 1.35, 1, 0.75, 0.55, 0.35, 0.04] if { native_size_category == "series" } else := [6, 2.5, 2.1, 1.6, 1.2, 0.85, 0.65, 0.5, 0.3, 0.04]
native_size_upper_multiplier := 1.5 if { input.release.video_codec == "AV1" } else := 1
native_size_threshold(index) := native_size_thresholds[index] * native_size_upper_multiplier if { index <= 5 } else := native_size_thresholds[index]
native_size_code(ratio) := "size_excessive_for_quality" if { ratio >= native_size_threshold(1) } else := "size_massive_for_quality" if { ratio >= native_size_threshold(2) } else := "size_very_large_for_quality" if { ratio >= native_size_threshold(3) } else := "size_large_for_quality" if { ratio >= native_size_threshold(4) } else := "size_expected_for_quality" if { ratio >= native_size_threshold(5) } else := "size_slightly_small_for_quality" if { ratio >= native_size_threshold(6) } else := "size_small_for_quality" if { ratio >= native_size_threshold(7) } else := "size_very_small_for_quality" if { ratio >= native_size_threshold(8) } else := "size_tiny_for_quality"

native_size_member_basis if {
  native_size_total_ratio < native_size_threshold(8)
  native_size_member_count > 1
  native_size_member_ratio >= native_size_threshold(7)
  native_size_member_ratio < native_size_threshold(2)
}
score_entry[native_size_code(native_size_member_ratio)] := 0 if { input.release.size_bytes != null; input.release.size_bytes > 0; native_size_member_basis }
score_entry["size_pack_member_basis"] := 0 if { input.release.size_bytes != null; input.release.size_bytes > 0; native_size_member_basis }
score_entry[native_size_code(native_size_total_ratio)] := 0 if { input.release.size_bytes != null; input.release.size_bytes > 0; not native_size_member_basis }