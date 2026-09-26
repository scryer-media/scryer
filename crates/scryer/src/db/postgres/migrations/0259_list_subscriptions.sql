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
    id text PRIMARY KEY NOT NULL,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider text NOT NULL,
    external_user_id text NOT NULL,
    username text NOT NULL,
    display_name text,
    credential_encrypted text NOT NULL,
    -- 'active' | 'expired' | 'revoked'
    status text NOT NULL DEFAULT 'active',
    error_message text,
    linked_at timestamptz NOT NULL,
    last_used_at timestamptz,
    last_refresh_at timestamptz,
    updated_at timestamptz NOT NULL
);
CREATE UNIQUE INDEX idx_user_list_accounts_identity
    ON user_list_accounts(user_id, provider, external_user_id);

CREATE TABLE list_subscriptions (
    id text PRIMARY KEY NOT NULL,
    -- 'public' | 'personal'
    scope text NOT NULL,
    owner_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider text NOT NULL,
    source_type text NOT NULL,
    source_params_json text NOT NULL DEFAULT '{}',
    -- 'smg_chart' | 'provider_fetch'
    source_origin text NOT NULL,
    chart_key text,
    chart_scope text,
    name text NOT NULL,
    provider_url text,
    kinds_json text NOT NULL DEFAULT '[]',
    enabled boolean NOT NULL DEFAULT true,
    -- 'search' | 'add' | 'hold' | 'request' | 'discover'
    mode text NOT NULL,
    filters_json text NOT NULL DEFAULT '[]',
    max_per_sync integer,
    -- 'keep' | 'log' | 'unmonitor' | 'tag'
    on_leave text NOT NULL DEFAULT 'keep',
    interval_seconds bigint NOT NULL,
    credential_id text REFERENCES user_list_accounts(id) ON DELETE SET NULL,
    -- 'ok' | 'new' | 'fail' | 'off'
    sync_state text NOT NULL DEFAULT 'new',
    last_sync_at timestamptz,
    next_sync_at timestamptz,
    error_message text,
    error_at timestamptz,
    paused_until timestamptz,
    fetch_fingerprint text,
    count_total bigint NOT NULL DEFAULT 0,
    count_in_library bigint NOT NULL DEFAULT 0,
    count_added bigint NOT NULL DEFAULT 0,
    count_requested bigint NOT NULL DEFAULT 0,
    count_held bigint NOT NULL DEFAULT 0,
    count_filtered bigint NOT NULL DEFAULT 0,
    count_excluded bigint NOT NULL DEFAULT 0,
    count_unresolved bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL
);
CREATE INDEX idx_list_subscriptions_owner ON list_subscriptions(owner_user_id);
CREATE INDEX idx_list_subscriptions_due ON list_subscriptions(enabled, next_sync_at);

-- One routing card per kind a subscription can contain.
CREATE TABLE list_subscription_routes (
    subscription_id text NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    kind text NOT NULL,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    quality_profile_id text,
    root_folder_id text,
    monitor_type text NOT NULL,
    min_availability text,
    use_season_folders boolean,
    release_numbering text,
    tags_json text NOT NULL DEFAULT '[]',
    PRIMARY KEY (subscription_id, kind)
);

CREATE TABLE list_memberships (
    subscription_id text NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    item_key text NOT NULL,
    rank bigint,
    season integer,
    display_title text,
    year integer,
    external_ids_json text NOT NULL DEFAULT '[]',
    smg_title_id bigint,
    title_id text,
    request_id text,
    kind text NOT NULL,
    state text NOT NULL,
    state_reason text,
    added_by_list boolean NOT NULL DEFAULT false,
    first_seen_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    left_at timestamptz,
    left_handled boolean NOT NULL DEFAULT false,
    PRIMARY KEY (subscription_id, item_key)
);
CREATE INDEX idx_list_memberships_title ON list_memberships(title_id);
CREATE INDEX idx_list_memberships_request ON list_memberships(request_id);

CREATE TABLE list_exclusions (
    id text PRIMARY KEY NOT NULL,
    kind text NOT NULL,
    display_title text NOT NULL,
    year integer,
    -- 'all_lists' | 'list'
    scope text NOT NULL,
    subscription_id text REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    created_by_user_id text REFERENCES users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL
);
CREATE INDEX idx_list_exclusions_subscription ON list_exclusions(subscription_id);

CREATE TABLE list_exclusion_external_ids (
    exclusion_id text NOT NULL REFERENCES list_exclusions(id) ON DELETE CASCADE,
    source text NOT NULL,
    value text NOT NULL,
    PRIMARY KEY (exclusion_id, source, value)
);
CREATE INDEX idx_list_exclusion_external_ids_lookup
    ON list_exclusion_external_ids(source, value);

CREATE TABLE user_list_policies (
    user_id text PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- 'auto' | 'approval' | 'none'
    policy text NOT NULL,
    updated_by_user_id text REFERENCES users(id) ON DELETE SET NULL,
    updated_at timestamptz NOT NULL
);

CREATE TABLE list_sync_runs (
    id text PRIMARY KEY NOT NULL,
    subscription_id text NOT NULL REFERENCES list_subscriptions(id) ON DELETE CASCADE,
    job_run_id text,
    started_at timestamptz NOT NULL,
    finished_at timestamptz,
    -- 'succeeded' | 'failed' | 'skipped'
    outcome text NOT NULL,
    counts_json text NOT NULL DEFAULT '{}',
    error_message text
);
CREATE INDEX idx_list_sync_runs_subscription
    ON list_sync_runs(subscription_id, started_at);

-- Where a media request came from. A list-originated request carries the
-- subscription that submitted it; the column is not a foreign key so the
-- request outlives an unfollowed list.
-- 'manual' | 'public_list' | 'personal_list'
ALTER TABLE media_requests ADD COLUMN origin_kind text NOT NULL DEFAULT 'manual';
ALTER TABLE media_requests ADD COLUMN origin_subscription_id text;
