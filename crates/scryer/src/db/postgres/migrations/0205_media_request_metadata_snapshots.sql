-- Retain the provider metadata that was visible when a request was submitted.
-- A single JSON snapshot keeps rating values, source attribution, vote counts,
-- and provider URLs internally consistent.
ALTER TABLE media_requests
    ADD COLUMN rating_summary_json TEXT NOT NULL DEFAULT '{"rating":null,"rating_sources":[],"external_ratings":[]}';
