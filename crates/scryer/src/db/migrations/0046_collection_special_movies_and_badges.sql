ALTER TABLE collections ADD COLUMN interstitial_association_confidence TEXT;
ALTER TABLE collections ADD COLUMN interstitial_continuity_status TEXT;
ALTER TABLE collections ADD COLUMN interstitial_movie_form TEXT;
ALTER TABLE collections ADD COLUMN interstitial_confidence TEXT;
ALTER TABLE collections ADD COLUMN interstitial_signal_summary TEXT;
ALTER TABLE collections ADD COLUMN special_movies_json TEXT NOT NULL DEFAULT '[]';
