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
  "Groups",
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
  {
    id: "block-exe-torrents",
    title: "No .exe torrents",
    description: "Strongly penalize torrents whose title names an executable file extension",
    category: "Torrent",
    regoSource: `import rego.v1

blocked_extensions := ["exe", "msi", "bat", "lnk"]

score_entry["blocked_file_extension"] := scryer.block_score() if {
    input.release.extra.protocol == "torrent"
    some part in split(lower(input.release.raw_title), ".")
    part in blocked_extensions
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
    id: "prefer-hevc",
    title: "Prefer HEVC",
    description: "Boost HEVC releases by 100 points and penalize x264 at 4K by 200",
    category: "Quality",
    regoSource: `import rego.v1

score_entry["hevc_bonus"] := 100 if {
    scryer.normalize_codec(input.release.video_codec) == "H.265"
}

score_entry["x264_4k_penalty"] := -200 if {
    input.release.quality == "2160P"
    scryer.normalize_codec(input.release.video_codec) == "H.264"
}`,
  },

  // ── Size ───────────────────────────────────────────────────────
  {
    id: "size-limits",
    title: "Prefer compact releases, penalize oversized",
    description: "Boost releases under 5 GiB; apply -10000 above 100 GiB",
    category: "Size",
    regoSource: `import rego.v1

score_entry["compact_bonus"] := 150 if {
    input.release.size_bytes != null
    scryer.size_gib(input.release.size_bytes) < 5
}

score_entry["too_large"] := scryer.block_score() if {
    input.release.size_bytes != null
    scryer.size_gib(input.release.size_bytes) > 100
}`,
  },

  // ── Audio ──────────────────────────────────────────────────────
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

  // ── Groups ─────────────────────────────────────────────────────
  {
    id: "release-group-scores",
    title: "Prefer release groups",
    description: "Boost releases from the groups you list; use a negative score to penalize them instead",
    category: "Groups",
    regoSource: `import rego.v1

preferred_groups := ["examplegroup", "samplesubs", "placeholderrip"]

score_entry["preferred_release_group"] := 400 if {
    input.release.release_group != null
    input.release.release_group != ""
    lower(input.release.release_group) in preferred_groups
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
