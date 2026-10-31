-- One row per indexer and media facet: the distinct titles in a row whose
-- background searches on that indexer came back empty, and the window during
-- which background searches leave the indexer out of that facet.
CREATE TABLE indexer_facet_learning (
    indexer_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    empty_title_ids TEXT NOT NULL DEFAULT '[]',
    suppressed_until TEXT,
    reprobe_claimed_at TEXT,
    revision INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (indexer_id, facet)
);
