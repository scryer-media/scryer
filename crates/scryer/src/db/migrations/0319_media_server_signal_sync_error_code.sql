-- The stable reason a watch-history sweep was incomplete, stored beside the
-- sentence in `last_error` so clients explain the condition without reading
-- prose. Rows written before this column existed keep a NULL code and are read
-- through their sentence alone.
ALTER TABLE media_server_signal_sync_state ADD COLUMN last_error_code TEXT;
