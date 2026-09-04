-- An OAuth client now stores what it is for. Only a `jellyfin_plugin` client may
-- bind the `jellyfin-link` scope to a Jellyfin media-server connection, so the
-- settings panel and the authorization path both read this column instead of
-- guessing from the shape of the callback URL.
ALTER TABLE oauth_client_registrations
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'custom'
        CHECK (kind IN ('custom', 'jellyfin_plugin'));

-- One-time claim of the rows the settings panel created for the Jellyfin plugin
-- before this column existed. Their callback shape and display name are the only
-- evidence they carry, so that heuristic lives here and nowhere else.
UPDATE oauth_client_registrations
   SET kind = 'jellyfin_plugin'
 WHERE display_name = 'Jellyfin Scryer plugin'
   AND client_id IN (
        SELECT client_id
          FROM oauth_client_redirect_uris
         GROUP BY client_id
        HAVING COUNT(*) = 1
           AND MAX(redirect_uri) LIKE '%/Scryer/Auth/Callback'
   );
