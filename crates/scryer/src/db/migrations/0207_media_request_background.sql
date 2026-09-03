-- Keep the provider's background art alongside the poster so request cards
-- can show it without re-reading metadata.
ALTER TABLE media_requests
    ADD COLUMN background_url TEXT;
