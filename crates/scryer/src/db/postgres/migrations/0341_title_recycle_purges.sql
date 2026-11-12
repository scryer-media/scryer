-- Titles deleted with recycle-bin entries that housekeeping still has to
-- remove for good, with the library whose bins hold them.
--
-- A title delete records the title here in the same transaction that deletes
-- its row, instead of removing its recycle entries itself. The next
-- housekeeping pass removes those entries from the library's bins while it
-- lists them anyway, and drops the row once none is left. A row whose entry
-- could not be removed (held, failed or quarantined) stays for the next pass,
-- until it is older than the history retention period: then the row is
-- dropped and its entries are left to the bin's own retention.
-- The table starts empty and holds at most one row per deleted title.
CREATE TABLE IF NOT EXISTS title_recycle_purges (
    title_id TEXT PRIMARY KEY NOT NULL,
    library_id TEXT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL
);

-- Housekeeping reads the oldest records first and drops the expired ones.
CREATE INDEX IF NOT EXISTS idx_title_recycle_purges_recorded_at
    ON title_recycle_purges (recorded_at, title_id);
