-- PostgreSQL half of the release-day index: the days a title's stored release
-- dates fall on, one row per distinct day, so a release-date window is an
-- index range read. The derivation and the meaning of `release_day = ''`
-- ("the store cannot judge this title, always keep it") match SQLite.
--
-- The day is compared byte by byte, as SQLite compares it, whatever collation
-- the database was created with.
CREATE TABLE IF NOT EXISTS title_release_days (
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    release_day TEXT COLLATE "C" NOT NULL,
    PRIMARY KEY (title_id, release_day)
);

-- Every day one title's three source columns contribute. Text that is not
-- valid JSON yields '' rather than an error, so neither the migration nor a
-- title write can fail on a stored document. Only the cast sits inside the
-- handler: it writes nothing, so the block costs no transaction id.
CREATE OR REPLACE FUNCTION title_release_day_candidates(
    first_aired TEXT,
    digital_release_date TEXT,
    movie_release_dates_json TEXT
)
RETURNS SETOF TEXT AS $$
DECLARE
    release_dates JSONB;
BEGIN
    RETURN QUERY
    SELECT CASE WHEN candidate.value ~ '^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
                THEN SUBSTR(candidate.value, 1, 10) ELSE '' END
      FROM (VALUES (first_aired), (digital_release_date)) AS candidate(value)
     WHERE TRIM(candidate.value) <> '';

    IF movie_release_dates_json IS NULL THEN
        RETURN;
    END IF;

    BEGIN
        release_dates := movie_release_dates_json::jsonb;
    EXCEPTION WHEN OTHERS THEN
        RETURN NEXT '';
        RETURN;
    END;

    RETURN QUERY
    SELECT CASE WHEN release_date.value ~ '^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'
                THEN SUBSTR(release_date.value, 1, 10) ELSE '' END
      FROM jsonb_each(
               CASE WHEN jsonb_typeof(release_dates) = 'object'
                    THEN release_dates
                    ELSE jsonb_build_object() END
           ) AS release_list(key, value)
     CROSS JOIN LATERAL jsonb_array_elements_text(
               CASE WHEN jsonb_typeof(release_list.value) = 'array'
                    THEN release_list.value
                    ELSE jsonb_build_array() END
           ) AS release_date(value)
     WHERE release_list.key IN ('theatrical_dates', 'digital_dates', 'physical_dates',
                                'any_market_theatrical_dates', 'any_market_digital_dates',
                                'any_market_physical_dates')
       AND TRIM(release_date.value) <> '';
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION title_release_days_refresh()
RETURNS TRIGGER AS $$
BEGIN
    DELETE FROM title_release_days WHERE title_id = NEW.id;
    INSERT INTO title_release_days (title_id, release_day)
    SELECT NEW.id, release_day
      FROM title_release_day_candidates(
               NEW.first_aired, NEW.digital_release_date, NEW.movie_release_dates_json
           ) AS release_day
    ON CONFLICT DO NOTHING;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS titles_insert_release_days ON titles;
CREATE TRIGGER titles_insert_release_days
AFTER INSERT ON titles
FOR EACH ROW
EXECUTE FUNCTION title_release_days_refresh();

-- A write that leaves all three source columns as they were touches nothing
-- here.
DROP TRIGGER IF EXISTS titles_update_release_days ON titles;
CREATE TRIGGER titles_update_release_days
AFTER UPDATE ON titles
FOR EACH ROW
WHEN (OLD.first_aired IS DISTINCT FROM NEW.first_aired
   OR OLD.digital_release_date IS DISTINCT FROM NEW.digital_release_date
   OR OLD.movie_release_dates_json IS DISTINCT FROM NEW.movie_release_dates_json)
EXECUTE FUNCTION title_release_days_refresh();

-- Titles stored before the triggers existed.
INSERT INTO title_release_days (title_id, release_day)
SELECT titles.id, release_day
  FROM titles
 CROSS JOIN LATERAL title_release_day_candidates(
           titles.first_aired, titles.digital_release_date, titles.movie_release_dates_json
       ) AS release_day
ON CONFLICT DO NOTHING;

-- What a window reads. Built after the rows above are in, in one pass,
-- instead of being maintained through every one of them.
CREATE INDEX IF NOT EXISTS idx_title_release_days_day
    ON title_release_days(release_day, title_id);
