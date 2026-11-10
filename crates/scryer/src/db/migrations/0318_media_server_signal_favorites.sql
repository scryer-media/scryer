-- Favourites as a watch signal. Jellyfin and Emby report `UserData.IsFavorite`
-- beside played state; it is stored on the same observation row so a person's
-- favourite is an intent signal comparable to a watchlist entry. Plex has no
-- favourite, so its rows keep the default.
ALTER TABLE media_server_user_media_signals ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
