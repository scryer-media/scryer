ALTER TABLE titles
    ADD COLUMN smg_identity_backfill_attempt_count INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_titles_movie_smg_identity_backfill_candidates
    ON titles(facet, smg_identity_backfill_attempt_count, id);
