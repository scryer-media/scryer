-- Find the episodes that air inside a date range.
--
-- The calendar asks for every episode whose air date falls between two days.
-- With no index on `episodes.air_date`, answering meant reading every episode
-- in the library. This indexes the air date so the range reads only the
-- episodes inside it, already in air-date order.
CREATE INDEX IF NOT EXISTS idx_episodes_air_date
    ON episodes(air_date);
