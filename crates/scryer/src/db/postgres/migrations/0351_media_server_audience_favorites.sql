-- Favourites by every account a media server knows, linked to a Scryer user
-- or not, one row per account and item. They feed the favourite facts: the
-- anonymous "anyone favourited it" fact, and through the signal roster the
-- per-user ones. A sweep replaces an account's rows only when they changed,
-- keeps them when that account's read failed, and removes an account the
-- server no longer lists. Rows go with their connection.
--
-- A favourite that could not be placed on Scryer media keeps a row with no
-- title, so the account's read is held unknown rather than counting it as
-- nothing.
CREATE TABLE media_server_audience_favorites (
    connection_id text NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    external_account_id text NOT NULL CHECK (external_account_id <> ''),
    provider_item_id text NOT NULL CHECK (provider_item_id <> ''),
    kind text NOT NULL CHECK (kind IN ('movie', 'series', 'season', 'episode')),
    scryer_title_id text,
    scryer_episode_id text,
    season_number bigint,
    PRIMARY KEY (connection_id, external_account_id, provider_item_id)
);

CREATE INDEX idx_media_server_audience_favorites_title
    ON media_server_audience_favorites(scryer_title_id);

-- One row per listed account: whether its last favourites read counts. A
-- favourite fact is known false only while every listed account of every
-- favourite-capable connection has a recent read_at. A failed or unplaced
-- read clears read_at and records the reason code.
CREATE TABLE media_server_audience_favorite_reads (
    connection_id text NOT NULL
        REFERENCES media_server_connections(id) ON DELETE CASCADE,
    external_account_id text NOT NULL CHECK (external_account_id <> ''),
    sweep_started_at timestamp with time zone NOT NULL,
    read_at timestamp with time zone,
    failure_reason text,
    PRIMARY KEY (connection_id, external_account_id)
);
