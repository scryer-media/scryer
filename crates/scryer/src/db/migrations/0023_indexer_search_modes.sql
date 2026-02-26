ALTER TABLE indexers ADD COLUMN enable_interactive_search INTEGER NOT NULL DEFAULT 1;
ALTER TABLE indexers ADD COLUMN enable_auto_search INTEGER NOT NULL DEFAULT 1;
