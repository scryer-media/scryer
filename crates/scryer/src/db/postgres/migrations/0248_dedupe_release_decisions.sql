-- Collapse repeated identical release decisions. See the SQLite twin for what
-- was appending them and why the newest row of each group is the one to keep.

DELETE FROM release_decisions
WHERE id NOT IN (
    SELECT id
      FROM (
            SELECT id,
                   ROW_NUMBER() OVER (
                       PARTITION BY wanted_item_id,
                                    decision_code,
                                    COALESCE(release_url, ''),
                                    release_title,
                                    COALESCE(release_size_bytes, -1)
                       ORDER BY created_at DESC, id DESC
                   ) AS identity_rank
              FROM release_decisions
           ) ranked
     WHERE identity_rank = 1
);
