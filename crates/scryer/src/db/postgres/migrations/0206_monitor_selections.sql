-- PostgreSQL twin of migrations/0206_monitor_selections.sql.
-- Advanced monitoring stores an explicit set of seasons and canon series
-- movies per owner (a title, or a still-pending media request). Everything the
-- owner does not list stays unmonitored.
CREATE TABLE monitor_selections (
    owner_kind text NOT NULL,
    owner_id text NOT NULL,
    entry_kind text NOT NULL,
    entry_key text NOT NULL,
    label text,
    external_ids_json text NOT NULL DEFAULT '[]',
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT monitor_selections_pkey PRIMARY KEY (owner_kind, owner_id, entry_kind, entry_key),
    CONSTRAINT monitor_selections_owner_kind_check
        CHECK ((owner_kind = ANY (ARRAY['title'::text, 'media_request'::text]))),
    CONSTRAINT monitor_selections_entry_kind_check
        CHECK ((entry_kind = ANY (ARRAY['season'::text, 'series_movie'::text])))
);

CREATE INDEX idx_monitor_selections_owner
    ON monitor_selections (owner_kind, owner_id);

-- The monitor-type CHECK enumerates the pre-advanced values, so `advanced`
-- cannot be stored while it stands. Validation already lives in the application
-- layer (`normalize_requested_monitor_type`); drop the constraint to match the
-- rebuilt SQLite table.
ALTER TABLE media_requests
    DROP CONSTRAINT IF EXISTS media_requests_requested_monitor_type_check;
