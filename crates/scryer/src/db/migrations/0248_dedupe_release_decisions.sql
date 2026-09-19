-- Collapse repeated identical release decisions.
--
-- Every RSS cadence re-evaluated the same releases for the same scopes and
-- appended a byte-identical ledger row for each one: an idle 12k-title
-- instance wrote 4,720 rows (2.7 MB) in 110 minutes for 19 scopes, 4,704 of
-- them `queued_better_or_equal`. The store now updates the matching row in
-- place, keyed on (wanted_item_id, decision_code, release identity), so this
-- one-time pass reclaims what the append-only behaviour already wrote.
--
-- The newest row of each group survives: it carries the scores and
-- explanation a reader would have been shown, and every list read is ordered
-- `created_at DESC`.

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
