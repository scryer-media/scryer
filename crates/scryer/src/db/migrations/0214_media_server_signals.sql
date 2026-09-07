-- Media-server watch signals (RFC 137 section 7.3, "Normalized observation
-- model"). Provider-neutral from the first row: `provider` is stored per row
-- rather than implied, so the Emby and Plex adapters that follow write into
-- this same table without a migration.
--
-- Two rules from the RFC are expressed in the schema itself:
--
-- * `scryer_title_id` and `scryer_episode_id` are nullable because identity
--   mapping rule 4 retains ambiguous and unmatched observations without
--   applying them to a subject. An unmapped observation is a real record of
--   what a person watched; it is simply not attributed yet.
-- * `sync_generation` makes "no longer played" expressible. A sweep writes its
--   rows with a fresh generation and then deletes that participant's older
--   generations, so an item that has dropped out of the provider's played set
--   disappears instead of lingering as a stale `played = 1`.

CREATE TABLE media_server_user_media_signals (
    id TEXT PRIMARY KEY NOT NULL,
    connection_id TEXT NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    -- Denormalized from the linked account at sync time. Kept as a plain
    -- column rather than a join so a later account unlink leaves the
    -- observation attributable to the identity that produced it.
    scryer_user_id TEXT,
    provider_item_id TEXT NOT NULL,
    -- 'movie' or 'episode'. Show-level rollups are computed by readers, never
    -- stored: a stored rollup would be a second source of truth that the next
    -- sweep could silently contradict.
    kind TEXT NOT NULL,
    scryer_title_id TEXT,
    scryer_episode_id TEXT,
    played INTEGER NOT NULL DEFAULT 0,
    play_count INTEGER NOT NULL DEFAULT 0,
    last_played_at TEXT,
    observed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    sync_generation INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- One observation per (connection, provider user, provider item). The provider
-- item id is the natural key on the provider's side; a second row for the same
-- triple would be two answers to one question.
CREATE UNIQUE INDEX idx_media_server_signals_participant_item
    ON media_server_user_media_signals(connection_id, external_user_id, provider_item_id);

CREATE INDEX idx_media_server_signals_title
    ON media_server_user_media_signals(scryer_title_id);

CREATE INDEX idx_media_server_signals_episode
    ON media_server_user_media_signals(scryer_episode_id);

-- Per-connection sync health (RFC 137 section 7.3: "Make signal freshness,
-- visibility, and lookup failures explicit"). One row per connection; the
-- sweep records its own failure here rather than losing it to a log line.
CREATE TABLE media_server_signal_sync_state (
    connection_id TEXT PRIMARY KEY NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    -- Snapshot of the connection's enabled flag as of the last sweep, so a
    -- reader can tell "nothing was collected because the connection is off"
    -- from "nothing was collected because the sweep failed".
    enabled INTEGER NOT NULL DEFAULT 0,
    last_started_at TEXT,
    last_success_at TEXT,
    last_error TEXT,
    participant_count INTEGER NOT NULL DEFAULT 0,
    signal_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
