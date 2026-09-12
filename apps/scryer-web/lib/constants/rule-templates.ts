export type RuleTemplate = {
  id: string;
  title: string;
  description: string;
  category: string;
  regoSource: string;
  appliedFacets?: string[];
};

export const RULE_TEMPLATE_CATEGORIES = [
  "Torrent",
  "Quality",
  "Size",
  "Audio",
  "Anime",
  "Penalties",
] as const;

export const RULE_TEMPLATES: RuleTemplate[] = [
  // ── Torrent ────────────────────────────────────────────────────
  {
    id: "freeleech-bonus",
    title: "Boost freeleech releases",
    description: "Add 500 points to freeleech releases from Torznab indexers",
    category: "Torrent",
    regoSource: `import rego.v1

score_entry["freeleech_bonus"] := 500 if {
    input.release.extra.freeleech == true
}`,
  },
  {
    id: "halfleech-bonus",
    title: "Boost half-leech releases",
    description: "Add 200 points to half-leech (50% download) releases",
    category: "Torrent",
    regoSource: `import rego.v1

score_entry["halfleech_bonus"] := 200 if {
    input.release.extra.downloadvolumefactor == 0.5
}`,
  },
  {
    id: "well-seeded-bonus",
    title: "Prefer well-seeded torrents",
    description: "Boost releases with 10+ seeders, penalize those with fewer than 3",
    category: "Torrent",
    regoSource: `import rego.v1

score_entry["well_seeded"] := 200 if {
    input.release.extra.seeders >= 10
}

score_entry["poorly_seeded"] := -300 if {
    input.release.extra.seeders != null
    input.release.extra.seeders < 3
}`,
  },

  // ── Quality ────────────────────────────────────────────────────
  {
    id: "prefer-web-dl",
    title: "Prefer WEB-DL over WEBRip",
    description: "Boost WEB-DL source releases by 100 points",
    category: "Quality",
    regoSource: `import rego.v1

score_entry["prefer_webdl"] := 100 if {
    scryer.normalize_source(input.release.source) == "WEB-DL"
}`,
  },
  {
    id: "prefer-x265",
    title: "Prefer x265/HEVC",
    description: "Boost HEVC/x265 releases by 100 points for better compression",
    category: "Quality",
    regoSource: `import rego.v1

score_entry["x265_bonus"] := 100 if {
    codec := scryer.normalize_codec(input.release.video_codec)
    codec == "H.265"
}`,
  },
  {
    id: "penalize-x264-4k",
    title: "Penalize x264 at 4K",
    description: "x264 at 4K is wasteful — penalize by 200 points",
    category: "Quality",
    regoSource: `import rego.v1

score_entry["x264_4k_penalty"] := -200 if {
    input.release.quality == "2160P"
    scryer.normalize_codec(input.release.video_codec) == "H.264"
}`,
  },

  // ── Size ───────────────────────────────────────────────────────
  {
    id: "block-oversized",
    title: "Penalize oversized releases (>100 GiB)",
    description: "Apply -10000 above 100 GiB; other scores can offset this penalty",
    category: "Size",
    regoSource: `import rego.v1

score_entry["too_large"] := scryer.block_score() if {
    scryer.size_gib(input.release.size_bytes) > 100
}`,
  },
  {
    id: "prefer-compact",
    title: "Prefer compact releases (<5 GiB)",
    description: "Boost releases under 5 GiB for bandwidth-conscious setups",
    category: "Size",
    regoSource: `import rego.v1

score_entry["compact_bonus"] := 150 if {
    input.release.size_bytes != null
    scryer.size_gib(input.release.size_bytes) < 5
}`,
  },
  {
    id: "block-tiny-releases",
    title: "Penalize suspiciously small releases",
    description: "Penalize releases under 100 MiB that are likely fakes or samples",
    category: "Size",
    regoSource: `import rego.v1

score_entry["too_small"] := scryer.block_score() if {
    input.release.size_bytes != null
    input.release.size_bytes > 0
    scryer.size_gib(input.release.size_bytes) < 0.1
}`,
  },

  // ── Audio ──────────────────────────────────────────────────────
  {
    id: "require-japanese-audio",
    title: "Prefer Japanese audio (post-download)",
    description: "Apply -10000 without Japanese audio. Use profile required languages for a mandatory requirement",
    category: "Audio",
    appliedFacets: ["anime"],
    regoSource: `import rego.v1

score_entry["no_japanese_audio"] := scryer.block_score() if {
    input.file != null
    not has_japanese_audio
}

has_japanese_audio if {
    some lang in input.file.audio_languages
    scryer.lang_matches(lang, "ja")
}`,
  },
  {
    id: "require-english-audio",
    title: "Prefer English audio (post-download)",
    description: "Apply -10000 without English audio. Use profile required languages for a mandatory requirement",
    category: "Audio",
    regoSource: `import rego.v1

score_entry["no_english_audio"] := scryer.block_score() if {
    input.file != null
    not has_english_audio
}

has_english_audio if {
    some lang in input.file.audio_languages
    scryer.lang_matches(lang, "en")
}`,
  },
  {
    id: "prefer-multi-audio",
    title: "Prefer multi-audio releases",
    description: "Boost releases that advertise dual audio or contain multiple audio tracks",
    category: "Audio",
    regoSource: `import rego.v1

score_entry["multi_audio_release_bonus"] := 200 if {
    input.release.is_dual_audio
}

score_entry["multi_audio_file_bonus"] := 200 if {
    not input.release.is_dual_audio
    input.file != null
    input.file.has_multiaudio
}`,
  },
  {
    id: "prefer-atmos-audio",
    title: "Prefer Atmos audio",
    description: "Add a small bonus for Atmos releases and a light penalty when Atmos is missing",
    category: "Audio",
    regoSource: `import rego.v1

score_entry["atmos_bonus"] := 100 if {
    input.release.is_atmos
}

score_entry["atmos_missing"] := -20 if {
    not input.release.is_atmos
}`,
  },

  // ── Anime ──────────────────────────────────────────────────────
  {
    id: "anime-group-preference",
    title: "Prefer specific anime release groups",
    description: "Boost releases from SubsPlease, Erai-raws, EMBER, and Yameii",
    category: "Anime",
    appliedFacets: ["anime"],
    regoSource: `import rego.v1

preferred_groups := {"subsplease", "erai-raws", "ember", "yameii"}

score_entry["preferred_anime_group"] := 400 if {
    input.release.release_group != null
    input.release.release_group != ""
    group := lower(input.release.release_group)
    preferred_groups[group]
}`,
  },
  {
    id: "block-mini-encodes",
    title: "Penalize mini encodes",
    description: "Penalize releases from known mini-encode groups",
    category: "Anime",
    appliedFacets: ["anime"],
    regoSource: `import rego.v1

mini_groups := {"judas", "bonkai77", "mini-encode", "minifreeza", "smallsizedanime"}

score_entry["block_mini_encode"] := scryer.block_score() if {
    input.release.release_group != null
    input.release.release_group != ""
    group := lower(input.release.release_group)
    mini_groups[group]
}`,
  },

  // ── Penalties ───────────────────────────────────────────────────
  {
    id: "block-old-releases",
    title: "Penalize releases older than 1 year",
    description: "Strongly penalize releases published more than 365 days ago",
    category: "Penalties",
    regoSource: `import rego.v1

score_entry["too_old"] := scryer.block_score() if {
    input.release.age_days > 365
}`,
  },
  {
    id: "block-password-protected",
    title: "Penalize password-protected releases",
    description: "Strongly penalize releases flagged as password protected",
    category: "Penalties",
    regoSource: `import rego.v1

score_entry["password_protected"] := scryer.block_score() if {
    input.release.is_password_protected == true
}`,
  },
  {
    id: "require-release-group",
    title: "Prefer a release group",
    description: "Penalize releases that do not expose a normalized release-group tag",
    category: "Penalties",
    regoSource: `import rego.v1

score_entry["missing_release_group"] := scryer.block_score() if {
    not input.release.has_release_group
}`,
  },
  {
    id: "block-obfuscated-retagged",
    title: "Penalize obfuscated or retagged releases",
    description: "Strongly penalize releases with normalized obfuscation or retagging signals",
    category: "Penalties",
    regoSource: `import rego.v1

score_entry["obfuscated_release"] := scryer.block_score() if {
    input.release.is_obfuscated
}

score_entry["retagged_release"] := scryer.block_score() if {
    input.release.is_retagged
}`,
  },
  {
    id: "block-low-quality-groups",
    title: "Penalize known low-quality groups",
    description: "Strongly penalize releases from groups known for poor quality",
    category: "Penalties",
    regoSource: `import rego.v1

blocked_groups := {"yify", "yts"}

score_entry["blocked_group"] := scryer.block_score() if {
    input.release.release_group != null
    input.release.release_group != ""
    group := lower(input.release.release_group)
    blocked_groups[group]
}`,
  },
  {
    id: "block-hardcoded-subs",
    title: "Penalize hardcoded subtitles",
    description: "Strongly penalize releases with hardcoded (burned-in) subtitles",
    category: "Penalties",
    regoSource: `import rego.v1

score_entry["hardcoded_subs"] := scryer.block_score() if {
    input.release.is_hardcoded_subs
}`,
  },
];
