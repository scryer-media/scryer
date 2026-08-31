CREATE TABLE download_submission_episode_links_0180 (
    download_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    PRIMARY KEY (download_id, episode_id),
    FOREIGN KEY (download_id) REFERENCES download_submissions(id) ON DELETE CASCADE
);

INSERT INTO download_submission_episode_links_0180 (download_id, episode_id)
SELECT submissions.id, links.episode_id
FROM download_submission_episode_links links
JOIN download_submissions submissions
    ON submissions.download_client_id = links.download_client_id
   AND submissions.download_client_type = links.download_client_type
   AND submissions.download_client_item_id = links.download_client_item_id;

DROP TABLE download_submission_episode_links;
ALTER TABLE download_submission_episode_links_0180
    RENAME TO download_submission_episode_links;

CREATE INDEX idx_download_submission_episode_links_episode
    ON download_submission_episode_links(episode_id);

ALTER TABLE download_submissions
    DROP CONSTRAINT download_submissions_download_client_id_download_client_typ_key;
ALTER TABLE download_submissions
    ADD CONSTRAINT download_submissions_id_fkey
    FOREIGN KEY (id) REFERENCES downloads(id);

ALTER TABLE download_identity_states
    ALTER COLUMN canonical_download_id SET NOT NULL;

ALTER TABLE imports
    ADD CONSTRAINT imports_canonical_download_id_fkey
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id);
ALTER TABLE download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_canonical_download_id_fkey
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id);
ALTER TABLE download_queue_commands
    ADD CONSTRAINT download_queue_commands_canonical_download_id_fkey
    FOREIGN KEY (canonical_download_id) REFERENCES downloads(id);

CREATE UNIQUE INDEX idx_download_client_bindings_active_locator_unique
    ON download_client_bindings(client_config_id, client_type_snapshot, native_item_id)
    WHERE native_item_id IS NOT NULL
      AND ended_at IS NULL;
