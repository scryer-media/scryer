-- Authorization codes are short lived and cannot be safely assigned a past
-- session epoch. Discard them before making the binding mandatory.
DELETE FROM oauth_authorization_codes;

ALTER TABLE oauth_authorization_codes
    ADD COLUMN auth_session_version TEXT NOT NULL;
