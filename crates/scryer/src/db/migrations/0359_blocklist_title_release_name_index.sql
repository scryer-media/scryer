-- Search-time exclusion reads a title's blocked releases by exact release
-- name. The unique name index covers only rows without an infohash and leads
-- with the indexer, so it cannot serve that lookup; this index serves it for
-- every row.
CREATE INDEX IF NOT EXISTS idx_blocklist_title_release_name
    ON blocklist (title_id, normalized_release_name);

-- Its leading column is the whole of the title index, which every title-scoped
-- read and the title foreign key can use instead.
DROP INDEX IF EXISTS idx_blocklist_title_id;
