-- Indexes for statements that read whole tables today, and removal of
-- indexes another index or key already covers.
--
-- Every dropped index is an exact duplicate or a leading prefix of an index,
-- primary key or unique constraint that stays, except the domain event time
-- index: the event type and time index below answers both statements that
-- used it. Dropping an index removes no rows.

-- Dashboard activity counts and event retention filter on type and time.
CREATE INDEX IF NOT EXISTS idx_domain_events_event_type_occurred_at
    ON domain_events(event_type, occurred_at);
DROP INDEX IF EXISTS idx_domain_events_occurred_at;

-- Imports by type and status, by download, and by client identity.
CREATE INDEX IF NOT EXISTS idx_imports_type_status_updated_at
    ON imports(import_type, status, updated_at);
CREATE INDEX IF NOT EXISTS idx_imports_canonical_download_id
    ON imports(canonical_download_id);
CREATE INDEX IF NOT EXISTS idx_imports_source_identity
    ON imports(COALESCE(source_client_id, ''), source_system, source_ref);

-- Wanted items paged by status, newest change first.
CREATE INDEX IF NOT EXISTS idx_wanted_items_status_updated_at
    ON wanted_items(status, updated_at);
DROP INDEX IF EXISTS idx_wanted_items_title;

-- Identity states walked by reason in download order.
CREATE INDEX IF NOT EXISTS idx_download_identity_states_reason_canonical
    ON download_identity_states(reason, canonical_download_id);
DROP INDEX IF EXISTS idx_download_identity_states_canonical_download_id;

-- A title's release decisions, newest first.
CREATE INDEX IF NOT EXISTS idx_release_decisions_title_created_at
    ON release_decisions(title_id, created_at);

-- A title's pending releases, newest first, and their removal.
CREATE INDEX IF NOT EXISTS idx_pending_releases_title_added
    ON pending_releases(title_id, added_at DESC);

-- Request counts and history per requester and per requested title.
CREATE INDEX IF NOT EXISTS idx_media_requests_requester_created
    ON media_requests(created_by_user_id, created_at, id);
CREATE INDEX IF NOT EXISTS idx_media_requests_fingerprint_created
    ON media_requests(identity_fingerprint, created_at, id);

-- History: two exact duplicates go; deleting a user clears its actor rows.
DROP INDEX IF EXISTS idx_history_title_time;
DROP INDEX IF EXISTS idx_history_type_time;
CREATE INDEX IF NOT EXISTS idx_history_events_actor_user
    ON history_events(actor_user_id)
    WHERE actor_user_id IS NOT NULL;

-- One library's catalog page in title order, either direction.
CREATE INDEX IF NOT EXISTS idx_titles_library_catalog_order_asc ON titles (
    library_id,
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX IF NOT EXISTS idx_titles_library_catalog_order_desc ON titles (
    library_id,
    catalog_sort_key COLLATE BINARY DESC,
    catalog_name_tie_key COLLATE BINARY DESC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year DESC,
    id COLLATE BINARY DESC
);

-- The catalog's needs-attention filter holds only stuck primary files.
CREATE INDEX IF NOT EXISTS idx_media_files_title_attention
    ON media_files(title_id)
    WHERE role = 'primary' AND scan_status IN ('scan_failed', 'review_required');
DROP INDEX IF EXISTS idx_media_files_title;

-- Finishing a search session updates its runs.
CREATE INDEX IF NOT EXISTS idx_indexer_search_runs_session
    ON indexer_search_runs(search_session_id);

-- Recent request rule decisions, with and without an outcome filter.
CREATE INDEX IF NOT EXISTS idx_request_rule_decisions_evaluated
    ON request_rule_decisions(evaluated_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_request_rule_decisions_outcome_evaluated
    ON request_rule_decisions(effective_outcome, evaluated_at DESC, id DESC);

-- Evaluation run history across every rule.
CREATE INDEX IF NOT EXISTS idx_maintenance_evaluation_runs_started
    ON maintenance_evaluation_runs(started_at DESC, id);

-- Discovery child rows reached through their parent's key.
CREATE INDEX IF NOT EXISTS idx_discovery_section_items_item
    ON discovery_section_items(item_id);
CREATE INDEX IF NOT EXISTS idx_discovery_items_discovery_title
    ON discovery_items(discovery_title_id);
CREATE INDEX IF NOT EXISTS idx_discovery_items_tombstoned_by_run
    ON discovery_items(tombstoned_by_run_id)
    WHERE tombstoned_by_run_id IS NOT NULL;

-- Covered by a key or a longer index that stays.
DROP INDEX IF EXISTS idx_discovery_titles_key_language;
DROP INDEX IF EXISTS idx_discovery_items_run;
DROP INDEX IF EXISTS idx_discovery_item_rank_components_item;
DROP INDEX IF EXISTS idx_discovery_title_source_tags_title;
DROP INDEX IF EXISTS idx_title_search_terms_title_id;
DROP INDEX IF EXISTS idx_indexer_search_run_sources_run;
DROP INDEX IF EXISTS idx_indexer_search_learning_title;
DROP INDEX IF EXISTS idx_external_subtitle_probe_cache_media_file;
DROP INDEX IF EXISTS idx_subtitle_blocklist_media_file;
DROP INDEX IF EXISTS idx_manual_import_selection_candidates_selection;
DROP INDEX IF EXISTS idx_maintenance_action_steps_candidate;
DROP INDEX IF EXISTS idx_monitor_selections_owner;
DROP INDEX IF EXISTS idx_title_images_title_kind;
DROP INDEX IF EXISTS idx_title_image_variants_image_variant;
