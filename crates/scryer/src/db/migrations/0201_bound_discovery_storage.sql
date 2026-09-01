CREATE TABLE title_recommendation_cards (
    discovery_title_id TEXT PRIMARY KEY NOT NULL,
    payload_version INTEGER NOT NULL DEFAULT 1,
    payload_blob BLOB,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO title_recommendation_cards (
    discovery_title_id,
    payload_version,
    payload_blob,
    created_at,
    updated_at
)
SELECT discovery_title_id,
       1,
       NULL,
       MIN(created_at),
       MAX(updated_at)
  FROM title_more_like_this_items
 GROUP BY discovery_title_id;

CREATE TABLE title_more_like_this_items_new (
    source_title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    discovery_title_id TEXT NOT NULL
        REFERENCES title_recommendation_cards(discovery_title_id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL DEFAULT 0,
    rank_score REAL,
    best_source TEXT,
    source_count INTEGER,
    edge_count INTEGER,
    relation_count INTEGER,
    source_subject_count INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (source_title_id, discovery_title_id)
);

INSERT INTO title_more_like_this_items_new (
    source_title_id,
    discovery_title_id,
    sort_index,
    rank_score,
    best_source,
    source_count,
    edge_count,
    relation_count,
    source_subject_count,
    created_at,
    updated_at
)
SELECT source_title_id,
       discovery_title_id,
       sort_index,
       rank_score,
       best_source,
       source_count,
       edge_count,
       relation_count,
       source_subject_count,
       created_at,
       updated_at
  FROM title_more_like_this_items;

DROP TABLE title_more_like_this_items;
ALTER TABLE title_more_like_this_items_new RENAME TO title_more_like_this_items;

CREATE INDEX idx_title_more_like_this_items_source_order
    ON title_more_like_this_items(source_title_id, sort_index ASC, rank_score DESC);
CREATE INDEX idx_title_more_like_this_items_title
    ON title_more_like_this_items(discovery_title_id);

ALTER TABLE discovery_sync_runs ADD COLUMN acknowledged_at TEXT;
UPDATE discovery_sync_runs
   SET acknowledged_at = COALESCE(updated_at, completed_at, created_at)
 WHERE raw_ack_json IS NOT NULL;
ALTER TABLE discovery_sync_runs DROP COLUMN raw_submit_json;
ALTER TABLE discovery_sync_runs DROP COLUMN raw_changes_json;
ALTER TABLE discovery_sync_runs DROP COLUMN raw_final_status_json;
ALTER TABLE discovery_sync_runs DROP COLUMN raw_ack_json;
