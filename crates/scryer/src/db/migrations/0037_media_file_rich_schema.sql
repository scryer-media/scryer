-- Rich media file schema: store parsed release metadata and acquisition provenance
-- alongside the existing media analysis (FFprobe) data.

ALTER TABLE media_files ADD COLUMN scene_name TEXT;
ALTER TABLE media_files ADD COLUMN release_group TEXT;
ALTER TABLE media_files ADD COLUMN source_type TEXT;
ALTER TABLE media_files ADD COLUMN resolution TEXT;
ALTER TABLE media_files ADD COLUMN video_codec_parsed TEXT;
ALTER TABLE media_files ADD COLUMN audio_codec_parsed TEXT;
ALTER TABLE media_files ADD COLUMN acquisition_score INTEGER;
ALTER TABLE media_files ADD COLUMN scoring_log TEXT;
ALTER TABLE media_files ADD COLUMN indexer_source TEXT;
ALTER TABLE media_files ADD COLUMN grabbed_release_title TEXT;
ALTER TABLE media_files ADD COLUMN grabbed_at TEXT;
ALTER TABLE media_files ADD COLUMN edition TEXT;
ALTER TABLE media_files ADD COLUMN original_file_path TEXT;
ALTER TABLE media_files ADD COLUMN release_hash TEXT;
