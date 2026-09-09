ALTER TABLE media_files ADD COLUMN analysis_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media_files ADD COLUMN analysis_attempt_revision INTEGER;
ALTER TABLE media_files ADD COLUMN analysis_attempted_at TEXT;
ALTER TABLE media_files ADD COLUMN analysis_attempt_source TEXT;
ALTER TABLE media_files ADD COLUMN analysis_attempt_status TEXT;
ALTER TABLE media_files ADD COLUMN analysis_attempt_json TEXT;
CREATE INDEX idx_media_files_analysis_refresh ON media_files(analysis_revision, analysis_attempted_at, id);
