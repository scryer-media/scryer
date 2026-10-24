ALTER TABLE user_ui_settings RENAME COLUMN palette TO previous_palette;
ALTER TABLE user_ui_settings ADD COLUMN palette TEXT NOT NULL DEFAULT 'CLASSIC'
    CHECK (palette IN ('CLASSIC', 'SCRYER_DEFAULT', 'SLATE', 'OCEAN', 'FOREST', 'PLUM', 'EMBER', 'GRAPHITE', 'PAPYRUS'));
UPDATE user_ui_settings SET palette = previous_palette;
ALTER TABLE user_ui_settings DROP COLUMN previous_palette;
