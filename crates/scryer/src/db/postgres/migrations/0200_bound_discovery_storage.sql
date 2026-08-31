CREATE TABLE title_recommendation_cards (
    discovery_title_id text PRIMARY KEY NOT NULL,
    payload_version integer NOT NULL DEFAULT 1,
    payload_blob bytea,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW()
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

ALTER TABLE title_more_like_this_items
    DROP CONSTRAINT IF EXISTS title_more_like_this_items_discovery_title_id_fkey;
ALTER TABLE title_more_like_this_items
    ADD CONSTRAINT title_more_like_this_items_recommendation_card_fkey
    FOREIGN KEY (discovery_title_id)
    REFERENCES title_recommendation_cards(discovery_title_id)
    ON DELETE CASCADE;

ALTER TABLE discovery_sync_runs ADD COLUMN acknowledged_at timestamptz;
UPDATE discovery_sync_runs
   SET acknowledged_at = COALESCE(updated_at, completed_at, created_at)
 WHERE raw_ack_json IS NOT NULL;
ALTER TABLE discovery_sync_runs DROP COLUMN raw_submit_json;
ALTER TABLE discovery_sync_runs DROP COLUMN raw_changes_json;
ALTER TABLE discovery_sync_runs DROP COLUMN raw_final_status_json;
ALTER TABLE discovery_sync_runs DROP COLUMN raw_ack_json;
