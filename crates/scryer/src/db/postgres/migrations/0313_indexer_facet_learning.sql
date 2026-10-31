-- One row per indexer and media facet: the distinct titles in a row whose
-- background searches on that indexer came back empty, and the window during
-- which background searches leave the indexer out of that facet.
CREATE TABLE indexer_facet_learning (
    indexer_id text NOT NULL,
    facet text NOT NULL,
    empty_title_ids text DEFAULT '[]' NOT NULL,
    suppressed_until timestamp with time zone,
    reprobe_claimed_at timestamp with time zone,
    revision bigint DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (indexer_id, facet)
);
