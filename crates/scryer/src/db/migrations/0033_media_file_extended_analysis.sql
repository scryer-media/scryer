ALTER TABLE media_files ADD COLUMN video_frame_rate TEXT;
ALTER TABLE media_files ADD COLUMN video_profile TEXT;
ALTER TABLE media_files ADD COLUMN audio_bitrate_kbps INTEGER;
ALTER TABLE media_files ADD COLUMN subtitle_codecs_json TEXT;
