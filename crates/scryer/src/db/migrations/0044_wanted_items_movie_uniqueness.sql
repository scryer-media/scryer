WITH ranked AS (
    SELECT
        id,
        title_id,
        FIRST_VALUE(id) OVER (
            PARTITION BY title_id
            ORDER BY
                CASE status
                    WHEN 'completed' THEN 0
                    WHEN 'grabbed' THEN 1
                    WHEN 'paused' THEN 2
                    ELSE 3
                END,
                COALESCE(updated_at, created_at) DESC,
                created_at DESC,
                rowid DESC
        ) AS keep_id,
        ROW_NUMBER() OVER (
            PARTITION BY title_id
            ORDER BY
                CASE status
                    WHEN 'completed' THEN 0
                    WHEN 'grabbed' THEN 1
                    WHEN 'paused' THEN 2
                    ELSE 3
                END,
                COALESCE(updated_at, created_at) DESC,
                created_at DESC,
                rowid DESC
        ) AS rank
    FROM wanted_items
    WHERE episode_id IS NULL
),
duplicates AS (
    SELECT id AS drop_id, keep_id
    FROM ranked
    WHERE rank > 1
)
UPDATE release_decisions
SET wanted_item_id = (
    SELECT keep_id
    FROM duplicates
    WHERE drop_id = release_decisions.wanted_item_id
)
WHERE wanted_item_id IN (SELECT drop_id FROM duplicates);

WITH ranked AS (
    SELECT
        id,
        title_id,
        FIRST_VALUE(id) OVER (
            PARTITION BY title_id
            ORDER BY
                CASE status
                    WHEN 'completed' THEN 0
                    WHEN 'grabbed' THEN 1
                    WHEN 'paused' THEN 2
                    ELSE 3
                END,
                COALESCE(updated_at, created_at) DESC,
                created_at DESC,
                rowid DESC
        ) AS keep_id,
        ROW_NUMBER() OVER (
            PARTITION BY title_id
            ORDER BY
                CASE status
                    WHEN 'completed' THEN 0
                    WHEN 'grabbed' THEN 1
                    WHEN 'paused' THEN 2
                    ELSE 3
                END,
                COALESCE(updated_at, created_at) DESC,
                created_at DESC,
                rowid DESC
        ) AS rank
    FROM wanted_items
    WHERE episode_id IS NULL
),
duplicates AS (
    SELECT id AS drop_id, keep_id
    FROM ranked
    WHERE rank > 1
)
UPDATE pending_releases
SET wanted_item_id = (
    SELECT keep_id
    FROM duplicates
    WHERE drop_id = pending_releases.wanted_item_id
)
WHERE wanted_item_id IN (SELECT drop_id FROM duplicates);

WITH ranked AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY title_id
            ORDER BY
                CASE status
                    WHEN 'completed' THEN 0
                    WHEN 'grabbed' THEN 1
                    WHEN 'paused' THEN 2
                    ELSE 3
                END,
                COALESCE(updated_at, created_at) DESC,
                created_at DESC,
                rowid DESC
        ) AS rank
    FROM wanted_items
    WHERE episode_id IS NULL
)
DELETE FROM wanted_items
WHERE id IN (SELECT id FROM ranked WHERE rank > 1);

CREATE UNIQUE INDEX IF NOT EXISTS idx_wanted_items_movie_unique
    ON wanted_items(title_id)
    WHERE episode_id IS NULL;
