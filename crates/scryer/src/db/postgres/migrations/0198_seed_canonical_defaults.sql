INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at) VALUES
    ('anime_default_library', 'anime', 'Anime', 'anime', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z'),
    ('movie_default_library', 'movie', 'Movies', 'movies', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z'),
    ('series_default_library', 'series', 'Series', 'series', true, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z')
ON CONFLICT DO NOTHING;

INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at)
SELECT roots.id, roots.library_id, roots.path, roots.normalized_path, true,
       '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z'
FROM (VALUES
    ('canonical_root_for_anime_default_library', 'anime_default_library', 'anime', 'anime', '/data/anime', '/data/anime'),
    ('canonical_root_for_movie_default_library', 'movie_default_library', 'movie', 'movies', '/data/movies', '/data/movies'),
    ('canonical_root_for_series_default_library', 'series_default_library', 'series', 'series', '/data/series', '/data/series')
) AS roots(id, library_id, facet, slug, path, normalized_path)
INNER JOIN libraries parent
        ON parent.id = roots.library_id
       AND parent.facet = roots.facet
       AND parent.slug = roots.slug
ON CONFLICT DO NOTHING;

INSERT INTO quality_profiles (id, name, scope, scope_id, archival_quality, allow_unknown_quality, atmos_preferred, dolby_vision_allowed, detected_hdr_allowed, prefer_remux, allow_bd_disk, allow_upgrades, created_at, prefer_dual_audio, required_audio_languages, scoring_config) VALUES
    ('1080p', '1080P', 'system', NULL, '1080P', false, true, true, true, true, false, true, '1970-01-01T00:00:00Z', false, '[]', '{}'),
    ('4k', '4K', 'system', NULL, '2160P', false, true, true, true, true, false, true, '1970-01-01T00:00:00Z', false, '[]', '{}')
ON CONFLICT DO NOTHING;

INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order, created_at) VALUES
    ('1080p', '1080P', 0, '1970-01-01T00:00:00Z'),
    ('1080p', '720P', 1, '1970-01-01T00:00:00Z'),
    ('4k', '1080P', 1, '1970-01-01T00:00:00Z'),
    ('4k', '2160P', 0, '1970-01-01T00:00:00Z'),
    ('4k', '720P', 2, '1970-01-01T00:00:00Z')
ON CONFLICT DO NOTHING;

INSERT INTO users (id, username, display_name, status, password_hash, passkey_public_key, locale, created_at, updated_at, last_login_at, account_kind, auth_session_version) VALUES
    ('00000000000000000000000000000001', 'admin', NULL, 'active', NULL, NULL, NULL, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z', NULL, 'local', NULL)
ON CONFLICT DO NOTHING;
