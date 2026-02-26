-- Background acquisition: wanted state tracking and release decision audit

CREATE TABLE IF NOT EXISTS wanted_items (
    id              TEXT PRIMARY KEY,
    title_id        TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    episode_id      TEXT REFERENCES episodes(id) ON DELETE CASCADE,
    media_type      TEXT NOT NULL,
    search_phase    TEXT NOT NULL DEFAULT 'primary',
    next_search_at  TEXT,
    last_search_at  TEXT,
    search_count    INTEGER NOT NULL DEFAULT 0,
    baseline_date   TEXT,
    status          TEXT NOT NULL DEFAULT 'wanted',
    grabbed_release TEXT,
    current_score   INTEGER,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE(title_id, episode_id)
);

CREATE INDEX IF NOT EXISTS idx_wanted_items_next_search
    ON wanted_items(status, next_search_at);
CREATE INDEX IF NOT EXISTS idx_wanted_items_title
    ON wanted_items(title_id);

CREATE TABLE IF NOT EXISTS release_decisions (
    id                  TEXT PRIMARY KEY,
    wanted_item_id      TEXT NOT NULL REFERENCES wanted_items(id) ON DELETE CASCADE,
    title_id            TEXT NOT NULL,
    release_title       TEXT NOT NULL,
    release_url         TEXT,
    release_size_bytes  INTEGER,
    decision_code       TEXT NOT NULL,
    candidate_score     INTEGER NOT NULL,
    current_score       INTEGER,
    score_delta         INTEGER,
    explanation_json    TEXT,
    created_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_release_decisions_wanted
    ON release_decisions(wanted_item_id, created_at DESC);
