-- Favourites are read on their own, so a favourite nobody played is stored as
-- an unplayed row. Jellyfin and Emby also let a person favourite a whole
-- series or one season; those rows use the kinds 'series' and 'season', and a
-- season row is placed by the provider's season number, kept here for
-- episodes too. Rows follow their connection and participant exactly as the
-- played rows do: the next successful sweep replaces them.
--
-- Lifecycle rules read a batch of titles' favourites, served by this index.

ALTER TABLE media_server_user_media_signals ADD COLUMN season_number bigint;

CREATE INDEX IF NOT EXISTS idx_media_server_signals_title_favorite
    ON media_server_user_media_signals(scryer_title_id, is_favorite);
