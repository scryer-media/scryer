CREATE TABLE IF NOT EXISTS indexers(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key_encrypted TEXT,
    rate_limit_seconds INTEGER,
    rate_limit_burst INTEGER,
    disabled_until TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    last_health_status TEXT,
    last_error_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS releases(
    id TEXT PRIMARY KEY,
    title_id TEXT,
    collection_id TEXT,
    episode_id TEXT,
    indexer_id TEXT,
    external_id TEXT,
    title TEXT NOT NULL,
    release_scope TEXT,
    download_hint TEXT,
    link TEXT,
    size_bytes INTEGER,
    published_at TEXT,
    language_raw TEXT,
    quality_label TEXT,
    raw_payload_json TEXT,
    parsed_payload_json TEXT,
    last_seen_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL,
    FOREIGN KEY (indexer_id) REFERENCES indexers(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS media_files(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    file_path TEXT NOT NULL UNIQUE,
    size_bytes INTEGER NOT NULL,
    quality_id TEXT,
    hash_sha256 TEXT,
    audio_languages_json TEXT,
    subtitle_languages_json TEXT,
    has_multiaudio INTEGER DEFAULT 0,
    scan_status TEXT NOT NULL DEFAULT 'pending',
    scan_error TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS file_episode_map(
    file_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    is_filler INTEGER DEFAULT 0,
    PRIMARY KEY (file_id, episode_id),
    FOREIGN KEY (file_id) REFERENCES media_files(id) ON DELETE CASCADE,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS quality_profiles(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    constraints_json TEXT NOT NULL,
    cutoff_rule_json TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS quality_rules(
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    name TEXT NOT NULL,
    attribute_key TEXT NOT NULL,
    attribute_value_json TEXT,
    operator TEXT,
    priority INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS rule_sets(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    constraints_json TEXT NOT NULL,
    cutoffs_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rule_set_assignments(
    rule_set_id TEXT NOT NULL,
    quality_rule_id TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    priority INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    PRIMARY KEY (rule_set_id, quality_rule_id),
    FOREIGN KEY (rule_set_id) REFERENCES rule_sets(id) ON DELETE CASCADE,
    FOREIGN KEY (quality_rule_id) REFERENCES quality_rules(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS policy_decisions(
    id TEXT PRIMARY KEY,
    rule_set_id TEXT NOT NULL,
    title_id TEXT,
    collection_id TEXT,
    episode_id TEXT,
    release_id TEXT,
    media_file_id TEXT,
    quality_profile_id TEXT,
    decision TEXT NOT NULL,
    score INTEGER,
    reason_json TEXT,
    decision_meta_json TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (rule_set_id) REFERENCES rule_sets(id) ON DELETE CASCADE,
    FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE SET NULL,
    FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE SET NULL,
    FOREIGN KEY (quality_profile_id) REFERENCES quality_profiles(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_releases_indexer_id
    ON releases (indexer_id);

CREATE INDEX IF NOT EXISTS idx_releases_title
    ON releases (title_id);

CREATE INDEX IF NOT EXISTS idx_releases_collection
    ON releases (collection_id);

CREATE INDEX IF NOT EXISTS idx_media_files_title
    ON media_files (title_id);

CREATE INDEX IF NOT EXISTS idx_file_episode_map_episode
    ON file_episode_map (episode_id);

CREATE INDEX IF NOT EXISTS idx_quality_rules_profile
    ON quality_rules (profile_id, priority);

CREATE INDEX IF NOT EXISTS idx_rule_set_assignments_rule
    ON rule_set_assignments (rule_set_id, quality_rule_id);

CREATE INDEX IF NOT EXISTS idx_rule_sets_scope
    ON rule_sets (scope, scope_id);

CREATE INDEX IF NOT EXISTS idx_policy_decisions_release
    ON policy_decisions (release_id);

CREATE INDEX IF NOT EXISTS idx_policy_decisions_file
    ON policy_decisions (media_file_id);

CREATE INDEX IF NOT EXISTS idx_policy_decisions_rule_set
    ON policy_decisions (rule_set_id);
