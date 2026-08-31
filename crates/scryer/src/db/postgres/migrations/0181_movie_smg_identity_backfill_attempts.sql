ALTER TABLE titles
    ADD COLUMN smg_identity_backfill_attempt_count BIGINT NOT NULL DEFAULT 0;

CREATE INDEX idx_titles_movie_smg_identity_backfill_candidates
    ON titles(facet, smg_identity_backfill_attempt_count, id);
