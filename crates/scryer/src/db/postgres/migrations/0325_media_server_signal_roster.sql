-- The links each media-server sweep actually read. One row per verified link,
-- written in the same transaction that stores the signals read through it and
-- removed with them when a later sweep no longer lists that identity.
--
-- The watch gate counts a link as observed only through a row here. A link
-- made after a sweep listed its participants has none until a sweep reads it,
-- so its user's missing plays are never reported as "did not watch". The table
-- starts empty: every link is unobserved until the first sweep after upgrade.
CREATE TABLE media_server_signal_roster (
    connection_id text NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    link_id text NOT NULL CHECK (link_id <> ''),
    external_user_id text NOT NULL,
    scryer_user_id text NOT NULL,
    -- The link's verified_at as the sweep saw it, kept for diagnosis. A login
    -- refreshes the link's value without changing whose plays are read.
    link_verified_at timestamp with time zone,
    sweep_started_at timestamp with time zone NOT NULL,
    read_at timestamp with time zone NOT NULL,
    PRIMARY KEY (connection_id, link_id)
);

-- Plays by every account a media server knows, linked to a Scryer user or
-- not, folded to one row per account and item. They feed only the anonymous
-- "anyone watched it" facts, so the account id keys the row and is never
-- reported. Each sweep replaces a connection's rows in one transaction.
CREATE TABLE media_server_audience_plays (
    connection_id text NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    external_account_id text NOT NULL CHECK (external_account_id <> ''),
    provider_item_id text NOT NULL CHECK (provider_item_id <> ''),
    kind text NOT NULL CHECK (kind IN ('movie', 'episode')),
    scryer_title_id text,
    scryer_episode_id text,
    play_count bigint NOT NULL DEFAULT 0,
    last_played_at timestamp with time zone,
    PRIMARY KEY (connection_id, external_account_id, provider_item_id)
);

CREATE INDEX idx_media_server_audience_plays_title
    ON media_server_audience_plays(scryer_title_id);

-- The last complete audience read per connection. The anonymous facts are
-- unknown for a connection without a recent row here; a failed read removes
-- the row so they stop answering at once.
CREATE TABLE media_server_audience_reads (
    connection_id text NOT NULL PRIMARY KEY
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    sweep_started_at timestamp with time zone NOT NULL,
    read_at timestamp with time zone NOT NULL,
    play_count bigint NOT NULL DEFAULT 0
);
