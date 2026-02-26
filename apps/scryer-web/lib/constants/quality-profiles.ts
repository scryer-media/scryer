export const QUALITY_TIER_CHOICES = [
  { value: "4320P", label: "8K (4320P)" },
  { value: "2160P", label: "4k (2160P)" },
  { value: "1440P", label: "1440P" },
  { value: "1080P", label: "1080P" },
  { value: "1080I", label: "1080i" },
  { value: "720P", label: "720P" },
  { value: "480P", label: "480P" },
  { value: "360P", label: "360P" },
] as const;

export const DEFAULT_QUALITY_PROFILE_QUALITY_TIERS = ["2160P", "1080P", "720P"] as const;

export const QUALITY_SOURCE_CHOICES = [
  { value: "WEB-DL", label: "WEB-DL" },
  { value: "BluRay", label: "BluRay" },
  { value: "HDTV", label: "HDTV" },
  { value: "DVD", label: "DVD" },
] as const;

export const VIDEO_CODEC_CHOICES = [
  { value: "H.264", label: "H.264" },
  { value: "H.265", label: "H.265" },
  { value: "AV1", label: "AV1" },
  { value: "VP9", label: "VP9" },
  { value: "VP8", label: "VP8" },
  { value: "XVID", label: "XVID" },
  { value: "x264", label: "x264 (encoding)" },
  { value: "x265", label: "x265 (encoding)" },
] as const;

export const AUDIO_CODEC_CHOICES = [
  { value: "AAC", label: "AAC" },
  { value: "AC3", label: "AC3" },
  { value: "DDP", label: "DDP" },
  { value: "DTS", label: "DTS" },
  { value: "EAC3", label: "EAC3" },
  { value: "FLAC", label: "FLAC" },
  { value: "OPUS", label: "OPUS" },
  { value: "TRUEHD", label: "TrueHD" },
] as const;
