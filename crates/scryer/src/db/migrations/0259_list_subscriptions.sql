-- Lists: public and personal list subscriptions, their memberships, the
-- instance-level exclusions, members' provider accounts, members' list
-- policies, and per-sync run records.
--
-- Two rules are expressed in the schema itself:
--
-- * Every table that belongs to a person cascades from `users`. A deleted
--   member leaves no subscription, account, or policy behind.
-- * A membership is keyed by the provider's own item id rather than by the
--   resolved title, so an item that has not resolved yet is still a row, and an
--   item that resolves later keeps its `first_seen_at`.
--
-- Credentials live encrypted in `user_list_accounts.credential_encrypted` with
-- the datastore key, never in plugin configuration.

CREATE TABLE user_list_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    display_name TEXT,
    credential_encrypted TEXT NOT NULL,
    -- 'active' | 'expired' | 'revoked'
    status TEXT NOT NULL DEFAULT 'active',
    error_message TEXT,
    linked_at TEXT NOT NULL,
    last_used_at TEXT,
    last_refresh_at TEXT,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_user_list_accounts_identity
    ON user_list_accounts(user_id, provider, external_user_id);

CREATE TABLE list_subscriptions (
    id TEXT PRIMARY KEY NOT NULL,
    -- 'public' | 'personal'
    scope TEXT NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_params_json TEXT NOT NULL DEFAULT '{}',
    -- 'smg_chart' | 'provider_fetch'
    source_origin TEXT NOT NULL,
    chart_key TEXT,
    chart_scope TEXT,
    name TEXT NOT NULL,
    provider_url TEXT,
    kinds_json TEXT NOT NULL DEFAULT '[]',
    enabled INTEGER NOT NULL DEFAULT 1,
    -- 'search' | 'add' | 'hold' | 'request' | 'discover'
    mode TEXT NOT NULL,
    filters_json TEXT NOT NULL DEFAULT '[]',
    max_per_sync INTEGER,
    -- 'keep' | 'log' | 'unmonitor' | 'tag'
    on_leave TEXT NOT NULL DEFAULT 'keep',
    interval_seconds INTEGER NOT NULL,
    credential_id TEXT REFERENCES user_list_accounts(id) ON DELETE SET NULL,
    -- 'ok' | 'new' | 'fail' | 'off'
    sync_state TEXT NOT NULL DEFAULT 'new',
    last_sync_at TEXT,
    next_sync_at TEXT,
    error_message TEXT,
    error_at TEXT,
    paused_until TEXT,
    fetch_fingerprint TEXT,
    count_total INTEGER NOT NULL DEFAULT 0,
    count_in_library INTEGER NOT NULL DEFAULT 0,
    count_added INTEGER NOT NULL DEFAULT 0,
    count_requested INTEGER NOT NULL DEFAULT 0,
    count_held INTEGER NOT NULL DEFAULT 0,
    count_filtered INTEGER NOT NULL DEFAULT 0,
    count_excluded INTEGER NOT NULL DEFAULT 0,
    count_unresolved INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_list_subscriptions_owner ON list_subscriptions(owner_user_id);
CREATE INDEX idx_list_subscriptions_due ON list_subscriptions(enabled, next_sync_at);

-- One routing card per kind a subscription can contain.
CREATE TABLE list_subscription_routes (
    subscription_id TEXT NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    quality_profile_id TEXT,
    root_folder_id TEXT,
    monitor_type TEXT NOT NULL,
    min_availability TEXT,
    use_season_folders INTEGER,
    release_numbering TEXT,
    tags_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (subscription_id, kind)
);

CREATE TABLE list_memberships (
    subscription_id TEXT NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    item_key TEXT NOT NULL,
    rank INTEGER,
    season INTEGER,
    display_title TEXT,
    year INTEGER,
    external_ids_json TEXT NOT NULL DEFAULT '[]',
    smg_title_id INTEGER,
    title_id TEXT,
    request_id TEXT,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    state_reason TEXT,
    added_by_list INTEGER NOT NULL DEFAULT 0,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    left_at TEXT,
    left_handled INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (subscription_id, item_key)
);
CREATE INDEX idx_list_memberships_title ON list_memberships(title_id);
CREATE INDEX idx_list_memberships_request ON list_memberships(request_id);

CREATE TABLE list_exclusions (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    display_title TEXT NOT NULL,
    year INTEGER,
    -- 'all_lists' | 'list'
    scope TEXT NOT NULL,
    subscription_id TEXT REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    created_by_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_list_exclusions_subscription ON list_exclusions(subscription_id);

CREATE TABLE list_exclusion_external_ids (
    exclusion_id TEXT NOT NULL REFERENCES list_exclusions(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (exclusion_id, source, value)
);
CREATE INDEX idx_list_exclusion_external_ids_lookup
    ON list_exclusion_external_ids(source, value);

CREATE TABLE user_list_policies (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- 'auto' | 'approval' | 'none'
    policy TEXT NOT NULL,
    updated_by_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE list_sync_runs (
    id TEXT PRIMARY KEY NOT NULL,
    subscription_id TEXT NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    job_run_id TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    -- 'succeeded' | 'failed' | 'skipped'
    outcome TEXT NOT NULL,
    counts_json TEXT NOT NULL DEFAULT '{}',
    error_message TEXT
);
CREATE INDEX idx_list_sync_runs_subscription
    ON list_sync_runs(subscription_id, started_at);

-- Where a media request came from. A list-originated request carries the
-- subscription that submitted it; the column is not a foreign key so the
-- request outlives an unfollowed list.
-- 'manual' | 'public_list' | 'personal_list'
ALTER TABLE media_requests ADD COLUMN origin_kind TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE media_requests ADD COLUMN origin_subscription_id TEXT;
