PRAGMA foreign_keys = OFF;

DROP TABLE IF EXISTS policy_decisions;
DROP TABLE IF EXISTS rule_set_assignments;
DROP TABLE IF EXISTS quality_rules;
DROP TABLE IF EXISTS rule_sets;
DROP TABLE IF EXISTS quality_profile_audio_codec_allowlist;
DROP TABLE IF EXISTS quality_profile_audio_codec_blocklist;
DROP TABLE IF EXISTS quality_profile_video_codec_allowlist;
DROP TABLE IF EXISTS quality_profile_video_codec_blocklist;
DROP TABLE IF EXISTS quality_profile_source_allowlist;
DROP TABLE IF EXISTS quality_profile_source_blocklist;
DROP TABLE IF EXISTS quality_profile_quality_tiers;
DROP TABLE IF EXISTS quality_profiles;

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS quality_profiles(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    archival_quality TEXT,
    allow_unknown_quality INTEGER NOT NULL DEFAULT 0,
    atmos_preferred INTEGER NOT NULL DEFAULT 0,
    dolby_vision_allowed INTEGER NOT NULL DEFAULT 0,
    detected_hdr_allowed INTEGER NOT NULL DEFAULT 1,
    prefer_remux INTEGER NOT NULL DEFAULT 0,
    allow_bd_disk INTEGER NOT NULL DEFAULT 0,
    allow_upgrades INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS quality_profile_quality_tiers(
    profile_id TEXT NOT NULL,
    quality_tier TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, quality_tier),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quality_profiles_scope
    ON quality_profiles (scope, scope_id);

CREATE INDEX IF NOT EXISTS idx_quality_profile_quality_tiers_profile
    ON quality_profile_quality_tiers (profile_id, sort_order);

CREATE TABLE IF NOT EXISTS quality_profile_source_allowlist(
    profile_id TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, source),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quality_profile_source_allowlist_profile
    ON quality_profile_source_allowlist (profile_id);

CREATE TABLE IF NOT EXISTS quality_profile_source_blocklist(
    profile_id TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, source),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quality_profile_source_blocklist_profile
    ON quality_profile_source_blocklist (profile_id);

CREATE TABLE IF NOT EXISTS quality_profile_video_codec_allowlist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quality_profile_video_codec_allowlist_profile
    ON quality_profile_video_codec_allowlist (profile_id);

CREATE TABLE IF NOT EXISTS quality_profile_video_codec_blocklist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quality_profile_video_codec_blocklist_profile
    ON quality_profile_video_codec_blocklist (profile_id);

CREATE TABLE IF NOT EXISTS quality_profile_audio_codec_allowlist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quality_profile_audio_codec_allowlist_profile
    ON quality_profile_audio_codec_allowlist (profile_id);

CREATE TABLE IF NOT EXISTS quality_profile_audio_codec_blocklist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quality_profile_audio_codec_blocklist_profile
    ON quality_profile_audio_codec_blocklist (profile_id);

INSERT OR IGNORE INTO quality_profiles
    (id, name, scope, scope_id, archival_quality, allow_unknown_quality, atmos_preferred,
     dolby_vision_allowed, detected_hdr_allowed, prefer_remux, allow_bd_disk, allow_upgrades, created_at)
VALUES
    (
        '4k',
        '4K',
        'system',
        NULL,
        '2160P',
        0,
        1,
        1,
        1,
        1,
        0,
        1,
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    ),
    (
        '1080p',
        '1080P',
        'system',
        NULL,
        '1080P',
        0,
        1,
        1,
        1,
        1,
        0,
        1,
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    );

INSERT OR IGNORE INTO quality_profile_quality_tiers(profile_id, quality_tier, sort_order)
VALUES
    ('4k', '2160P', 0),
    ('4k', '1080P', 1),
    ('4k', '720P', 2),
    ('1080p', '1080P', 0),
    ('1080p', '720P', 1);
