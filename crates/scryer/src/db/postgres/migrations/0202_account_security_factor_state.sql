ALTER TABLE totp_credentials
    ADD COLUMN attempt_window_started_at TIMESTAMPTZ;

ALTER TABLE totp_credentials
    ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0;

ALTER TABLE totp_enrollment_challenges
    ADD COLUMN auth_session_version TEXT;

ALTER TABLE webauthn_challenges
    ADD COLUMN auth_session_version TEXT;

DELETE FROM totp_enrollment_challenges
WHERE expires_at <= CURRENT_TIMESTAMP;

DELETE FROM totp_enrollment_challenges
WHERE id NOT IN (
    SELECT id
    FROM (
        SELECT
            id,
            ROW_NUMBER() OVER (
                PARTITION BY user_id
                ORDER BY expires_at DESC, created_at DESC, id DESC
            ) AS row_number
        FROM totp_enrollment_challenges
    ) AS numbered
    WHERE row_number = 1
);

CREATE UNIQUE INDEX totp_enrollment_challenges_one_active_per_user
    ON totp_enrollment_challenges (user_id);
