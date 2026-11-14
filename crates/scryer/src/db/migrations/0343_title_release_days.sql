-- The days a title's stored release dates fall on, one row per distinct day,
-- so a release-date window is an index range read instead of a scan that
-- pattern-tests two text columns and expands a JSON document per title.
--
-- A title's candidate values are `first_aired`, `digital_release_date` and
-- every member of the six date lists in `movie_release_dates_json`. A value
-- that starts with YYYY-MM-DD contributes those ten characters. A value that
-- is not blank but has no such start, and a document that is not NULL but is
-- not valid JSON, contribute the row `release_day = ''` instead: the store
-- cannot judge that title, so a window read always keeps it. A document that
-- is not an object, and a list member that is not an array, contribute
-- nothing.
--
-- The table is derived entirely from `titles` and kept by the triggers below,
-- so no writer of `titles` has to know about it. Rows leave with their title
-- through the cascade.
CREATE TABLE IF NOT EXISTS title_release_days (
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    release_day TEXT NOT NULL,
    PRIMARY KEY (title_id, release_day)
) WITHOUT ROWID;

-- A migration that rebuilds `titles` by copying it drops both triggers below
-- and must create them again.
--
-- Each trigger writes a title's days as one statement whose rows are already
-- distinct. A conflict clause on the insert would not do that job: when the
-- title is written by an upsert, SQLite replaces a trigger statement's own
-- conflict handling with the upsert's, and a repeated day would fail the
-- title write.
--
-- The delete is for a row that replaces one of the same id without the
-- cascade having run: the days it leaves behind would otherwise outlive it.
DROP TRIGGER IF EXISTS titles_insert_release_days;
CREATE TRIGGER titles_insert_release_days
AFTER INSERT ON titles
BEGIN
    DELETE FROM title_release_days WHERE title_id = NEW.id;

    INSERT INTO title_release_days (title_id, release_day)
    SELECT NEW.id, derived.release_day
      FROM (
            SELECT CASE WHEN candidate.value GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*'
                        THEN SUBSTR(candidate.value, 1, 10) ELSE '' END AS release_day
              FROM (
                    SELECT NEW.first_aired AS value
                    UNION ALL
                    SELECT NEW.digital_release_date
                    UNION ALL
                    SELECT release_date.value
                      FROM json_each(CASE WHEN json_valid(NEW.movie_release_dates_json)
                                          THEN NEW.movie_release_dates_json ELSE '{}' END) AS release_list,
                           json_each(CASE WHEN release_list.type = 'array'
                                          THEN release_list.value ELSE '[]' END) AS release_date
                     WHERE release_list.key IN ('theatrical_dates', 'digital_dates', 'physical_dates',
                                                'any_market_theatrical_dates', 'any_market_digital_dates',
                                                'any_market_physical_dates')
                   ) AS candidate
             WHERE TRIM(candidate.value) <> ''
            UNION
            SELECT ''
             WHERE NEW.movie_release_dates_json IS NOT NULL
               AND NOT json_valid(NEW.movie_release_dates_json)
           ) AS derived;
END;

-- A write that leaves all three source columns as they were touches nothing
-- here.
DROP TRIGGER IF EXISTS titles_update_release_days;
CREATE TRIGGER titles_update_release_days
AFTER UPDATE OF first_aired, digital_release_date, movie_release_dates_json ON titles
WHEN OLD.first_aired IS NOT NEW.first_aired
  OR OLD.digital_release_date IS NOT NEW.digital_release_date
  OR OLD.movie_release_dates_json IS NOT NEW.movie_release_dates_json
BEGIN
    DELETE FROM title_release_days WHERE title_id = NEW.id;

    INSERT INTO title_release_days (title_id, release_day)
    SELECT NEW.id, derived.release_day
      FROM (
            SELECT CASE WHEN candidate.value GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*'
                        THEN SUBSTR(candidate.value, 1, 10) ELSE '' END AS release_day
              FROM (
                    SELECT NEW.first_aired AS value
                    UNION ALL
                    SELECT NEW.digital_release_date
                    UNION ALL
                    SELECT release_date.value
                      FROM json_each(CASE WHEN json_valid(NEW.movie_release_dates_json)
                                          THEN NEW.movie_release_dates_json ELSE '{}' END) AS release_list,
                           json_each(CASE WHEN release_list.type = 'array'
                                          THEN release_list.value ELSE '[]' END) AS release_date
                     WHERE release_list.key IN ('theatrical_dates', 'digital_dates', 'physical_dates',
                                                'any_market_theatrical_dates', 'any_market_digital_dates',
                                                'any_market_physical_dates')
                   ) AS candidate
             WHERE TRIM(candidate.value) <> ''
            UNION
            SELECT ''
             WHERE NEW.movie_release_dates_json IS NOT NULL
               AND NOT json_valid(NEW.movie_release_dates_json)
           ) AS derived;
END;

-- Titles stored before the triggers existed.
INSERT OR IGNORE INTO title_release_days (title_id, release_day)
SELECT candidate.title_id,
       CASE WHEN candidate.value GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*'
            THEN SUBSTR(candidate.value, 1, 10) ELSE '' END
  FROM (
        SELECT id AS title_id, first_aired AS value FROM titles
        UNION ALL
        SELECT id, digital_release_date FROM titles
        UNION ALL
        SELECT titles.id, release_date.value
          FROM titles,
               json_each(CASE WHEN json_valid(titles.movie_release_dates_json)
                              THEN titles.movie_release_dates_json ELSE '{}' END) AS release_list,
               json_each(CASE WHEN release_list.type = 'array'
                              THEN release_list.value ELSE '[]' END) AS release_date
         WHERE release_list.key IN ('theatrical_dates', 'digital_dates', 'physical_dates',
                                    'any_market_theatrical_dates', 'any_market_digital_dates',
                                    'any_market_physical_dates')
       ) AS candidate
 WHERE TRIM(candidate.value) <> '';

INSERT OR IGNORE INTO title_release_days (title_id, release_day)
SELECT id, ''
  FROM titles
 WHERE movie_release_dates_json IS NOT NULL
   AND NOT json_valid(movie_release_dates_json);

-- What a window reads. Built after the rows above are in, in one pass,
-- instead of being maintained through every one of them.
CREATE INDEX IF NOT EXISTS idx_title_release_days_day
    ON title_release_days(release_day, title_id);
