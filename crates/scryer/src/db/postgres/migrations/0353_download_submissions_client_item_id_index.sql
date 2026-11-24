-- The failed-download check starts from the item ids a download client
-- reports as failed, without knowing which client type submitted them, so
-- it needs submissions by item id alone. The existing client-item index
-- leads with the client type and cannot serve that.
CREATE INDEX IF NOT EXISTS idx_download_submissions_client_item_id
    ON download_submissions (download_client_item_id);
