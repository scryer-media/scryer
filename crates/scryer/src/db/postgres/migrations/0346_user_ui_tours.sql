-- Guided tours (onboarding and release highlights) each user has finished or
-- skipped, so the web offers each one once. Tour ids are owned by the web's
-- tour registry; a row per tour ever shipped, removed with its user.
CREATE TABLE user_ui_tours (
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tour_id text NOT NULL,
    outcome text NOT NULL CHECK (outcome IN ('completed', 'skipped')),
    recorded_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (user_id, tour_id)
);

-- Accounts on an instance that already holds titles know their way around:
-- they are not shown onboarding, only the highlights of later releases. A
-- fresh install also has its bootstrap administrator by now, but no titles,
-- so that administrator still gets onboarding.
INSERT INTO user_ui_tours (user_id, tour_id, outcome)
SELECT id, 'onboarding', 'completed' FROM users
 WHERE EXISTS (SELECT 1 FROM titles);
