-- These values are immutable authorization-grant facts. A live connection
-- check still runs before linking, but this prevents a redirect or server
-- selection change from silently retargeting an already-approved grant.
ALTER TABLE oauth_authorization_codes
    ADD COLUMN jellyfin_connection_id TEXT NULL;
ALTER TABLE oauth_authorization_codes
    ADD COLUMN jellyfin_external_url TEXT NULL;
ALTER TABLE oauth_authorization_codes
    ADD COLUMN jellyfin_base_url TEXT NULL;
ALTER TABLE oauth_authorization_codes
    ADD COLUMN jellyfin_api_key_hash TEXT NULL;

ALTER TABLE oauth_refresh_grants
    ADD COLUMN redirect_uri TEXT NOT NULL DEFAULT '';
ALTER TABLE oauth_refresh_grants
    ADD COLUMN jellyfin_connection_id TEXT NULL;
ALTER TABLE oauth_refresh_grants
    ADD COLUMN jellyfin_external_url TEXT NULL;
ALTER TABLE oauth_refresh_grants
    ADD COLUMN jellyfin_base_url TEXT NULL;
ALTER TABLE oauth_refresh_grants
    ADD COLUMN jellyfin_api_key_hash TEXT NULL;
