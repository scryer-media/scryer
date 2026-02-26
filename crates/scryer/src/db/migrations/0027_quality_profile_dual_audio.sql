ALTER TABLE quality_profiles ADD COLUMN prefer_dual_audio INTEGER NOT NULL DEFAULT 0;
ALTER TABLE quality_profiles ADD COLUMN required_audio_languages TEXT NOT NULL DEFAULT '[]';
