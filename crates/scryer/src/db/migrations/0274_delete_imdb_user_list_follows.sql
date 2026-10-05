-- Scryer no longer follows public IMDb user lists: IMDb has no API for them
-- and the metadata gateway no longer proxies them. Remove every such follow
-- the way an unfollow does: the follow, its routing, memberships, list-scoped
-- exclusions and sync history go; every title, request and file a follow
-- touched stays. A request keeps its origin id, as it does after any unfollow.
-- Follows of IMDb charts are untouched; the gateway still serves those.
--
-- Children are deleted before the follow so the result does not depend on
-- foreign-key enforcement being on for the migration connection.
DELETE FROM list_exclusion_external_ids
WHERE exclusion_id IN (
    SELECT list_exclusions.id
    FROM list_exclusions
    JOIN list_subscriptions ON list_subscriptions.id = list_exclusions.subscription_id
    WHERE list_subscriptions.source_origin = 'smg_imdb_list'
);

DELETE FROM list_exclusions
WHERE subscription_id IN (
    SELECT id FROM list_subscriptions WHERE source_origin = 'smg_imdb_list'
);

DELETE FROM list_sync_runs
WHERE subscription_id IN (
    SELECT id FROM list_subscriptions WHERE source_origin = 'smg_imdb_list'
);

DELETE FROM list_memberships
WHERE subscription_id IN (
    SELECT id FROM list_subscriptions WHERE source_origin = 'smg_imdb_list'
);

DELETE FROM list_subscription_routes
WHERE subscription_id IN (
    SELECT id FROM list_subscriptions WHERE source_origin = 'smg_imdb_list'
);

DELETE FROM list_subscriptions WHERE source_origin = 'smg_imdb_list';
