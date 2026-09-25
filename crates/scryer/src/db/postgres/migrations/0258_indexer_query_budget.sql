-- Sustained query budget an indexer allows per minute. NULL means no budget
-- beyond the request interval.
ALTER TABLE indexers ADD COLUMN max_queries_per_minute bigint;
