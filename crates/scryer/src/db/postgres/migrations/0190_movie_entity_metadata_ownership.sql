ALTER TABLE title_metadata_rating_summaries DROP CONSTRAINT IF EXISTS title_metadata_rating_summaries_pkey;
ALTER TABLE title_metadata_rating_summaries ALTER COLUMN title_id DROP NOT NULL;
ALTER TABLE title_metadata_rating_summaries ADD COLUMN movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE title_metadata_rating_summaries ADD CONSTRAINT title_metadata_rating_summaries_owner_check
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL));

ALTER TABLE title_metadata_rating_sources DROP CONSTRAINT IF EXISTS title_metadata_rating_sources_pkey;
ALTER TABLE title_metadata_rating_sources ALTER COLUMN title_id DROP NOT NULL;
ALTER TABLE title_metadata_rating_sources ADD COLUMN movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE title_metadata_rating_sources ADD CONSTRAINT title_metadata_rating_sources_owner_check
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL));

ALTER TABLE title_metadata_external_ratings DROP CONSTRAINT IF EXISTS title_metadata_external_ratings_pkey;
ALTER TABLE title_metadata_external_ratings ALTER COLUMN title_id DROP NOT NULL;
ALTER TABLE title_metadata_external_ratings ADD COLUMN movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE title_metadata_external_ratings ADD CONSTRAINT title_metadata_external_ratings_owner_check
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL));

ALTER TABLE title_credits DROP CONSTRAINT IF EXISTS title_credits_pkey;
ALTER TABLE title_credits ALTER COLUMN title_id DROP NOT NULL;
ALTER TABLE title_credits ADD COLUMN movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE;
ALTER TABLE title_credits ADD CONSTRAINT title_credits_owner_check
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL));

CREATE UNIQUE INDEX idx_title_metadata_rating_summaries_title_owner
    ON title_metadata_rating_summaries(title_id) WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_summaries_movie_owner
    ON title_metadata_rating_summaries(movie_entity_id) WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_sources_title_owner
    ON title_metadata_rating_sources(title_id, source) WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_sources_movie_owner
    ON title_metadata_rating_sources(movie_entity_id, source) WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_external_ratings_title_owner
    ON title_metadata_external_ratings(title_id, source) WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_external_ratings_movie_owner
    ON title_metadata_external_ratings(movie_entity_id, source) WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_credits_title_owner
    ON title_credits(title_id, position) WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_credits_movie_owner
    ON title_credits(movie_entity_id, position) WHERE movie_entity_id IS NOT NULL;

CREATE INDEX idx_movie_entity_metadata_rating_sources_order
    ON title_metadata_rating_sources(movie_entity_id, sort_index ASC, source ASC);
CREATE INDEX idx_movie_entity_metadata_external_ratings_order
    ON title_metadata_external_ratings(movie_entity_id, sort_index ASC, source ASC);
CREATE INDEX idx_movie_entity_metadata_external_ratings_source_norm
    ON title_metadata_external_ratings(source, normalized, movie_entity_id);
CREATE INDEX idx_title_credits_movie_kind
    ON title_credits(movie_entity_id, kind);
