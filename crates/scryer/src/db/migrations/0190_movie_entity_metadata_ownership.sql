ALTER TABLE title_metadata_rating_summaries RENAME TO title_metadata_rating_summaries_old_0183;
CREATE TABLE title_metadata_rating_summaries (
    title_id TEXT REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE,
    rating REAL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))
);
INSERT INTO title_metadata_rating_summaries (title_id, rating, created_at, updated_at)
SELECT title_id, rating, created_at, updated_at
FROM title_metadata_rating_summaries_old_0183;
DROP TABLE title_metadata_rating_summaries_old_0183;

ALTER TABLE title_metadata_rating_sources RENAME TO title_metadata_rating_sources_old_0183;
CREATE TABLE title_metadata_rating_sources (
    title_id TEXT REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))
);
INSERT INTO title_metadata_rating_sources (
    title_id, source, sort_index, created_at, updated_at
)
SELECT title_id, source, sort_index, created_at, updated_at
FROM title_metadata_rating_sources_old_0183;
DROP TABLE title_metadata_rating_sources_old_0183;

ALTER TABLE title_metadata_external_ratings RENAME TO title_metadata_external_ratings_old_0183;
CREATE TABLE title_metadata_external_ratings (
    title_id TEXT REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    value REAL,
    score REAL,
    normalized REAL NOT NULL,
    votes INTEGER,
    url TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))
);
INSERT INTO title_metadata_external_ratings (
    title_id, source, sort_index, value, score, normalized, votes, url, created_at, updated_at
)
SELECT title_id, source, sort_index, value, score, normalized, votes, url, created_at, updated_at
FROM title_metadata_external_ratings_old_0183;
DROP TABLE title_metadata_external_ratings_old_0183;

ALTER TABLE title_credits RENAME TO title_credits_old_0183;
CREATE TABLE title_credits (
    title_id TEXT REFERENCES titles(id) ON DELETE CASCADE,
    movie_entity_id TEXT REFERENCES movie_entities(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    person_id TEXT NOT NULL,
    person_name TEXT NOT NULL DEFAULT '',
    person_original_name TEXT NOT NULL DEFAULT '',
    person_image_url TEXT NOT NULL DEFAULT '',
    person_source TEXT NOT NULL DEFAULT '',
    person_external_id TEXT NOT NULL DEFAULT '',
    character_name TEXT NOT NULL DEFAULT '',
    language TEXT NOT NULL DEFAULT '',
    billing_order INTEGER NOT NULL DEFAULT 0,
    episode_count INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK ((title_id IS NOT NULL) <> (movie_entity_id IS NOT NULL))
);
INSERT INTO title_credits (
    title_id, position, kind, person_id, person_name, person_original_name,
    person_image_url, person_source, person_external_id, character_name,
    language, billing_order, episode_count, created_at, updated_at
)
SELECT
    title_id, position, kind, person_id, person_name, person_original_name,
    person_image_url, person_source, person_external_id, character_name,
    language, billing_order, episode_count, created_at, updated_at
FROM title_credits_old_0183;
DROP TABLE title_credits_old_0183;

CREATE UNIQUE INDEX idx_title_metadata_rating_summaries_title_owner
    ON title_metadata_rating_summaries(title_id)
    WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_summaries_movie_owner
    ON title_metadata_rating_summaries(movie_entity_id)
    WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_sources_title_owner
    ON title_metadata_rating_sources(title_id, source)
    WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_rating_sources_movie_owner
    ON title_metadata_rating_sources(movie_entity_id, source)
    WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_external_ratings_title_owner
    ON title_metadata_external_ratings(title_id, source)
    WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_metadata_external_ratings_movie_owner
    ON title_metadata_external_ratings(movie_entity_id, source)
    WHERE movie_entity_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_credits_title_owner
    ON title_credits(title_id, position)
    WHERE title_id IS NOT NULL;
CREATE UNIQUE INDEX idx_title_credits_movie_owner
    ON title_credits(movie_entity_id, position)
    WHERE movie_entity_id IS NOT NULL;

CREATE INDEX idx_title_metadata_rating_sources_order
    ON title_metadata_rating_sources(title_id, sort_index ASC, source ASC);
CREATE INDEX idx_movie_entity_metadata_rating_sources_order
    ON title_metadata_rating_sources(movie_entity_id, sort_index ASC, source ASC);
CREATE INDEX idx_title_metadata_external_ratings_order
    ON title_metadata_external_ratings(title_id, sort_index ASC, source ASC);
CREATE INDEX idx_movie_entity_metadata_external_ratings_order
    ON title_metadata_external_ratings(movie_entity_id, sort_index ASC, source ASC);
CREATE INDEX idx_title_metadata_external_ratings_source_norm
    ON title_metadata_external_ratings(source, normalized, title_id);
CREATE INDEX idx_movie_entity_metadata_external_ratings_source_norm
    ON title_metadata_external_ratings(source, normalized, movie_entity_id);
CREATE INDEX idx_title_credits_title_kind
    ON title_credits(title_id, kind);
CREATE INDEX idx_title_credits_movie_kind
    ON title_credits(movie_entity_id, kind);
