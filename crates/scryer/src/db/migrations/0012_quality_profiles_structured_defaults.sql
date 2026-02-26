INSERT OR IGNORE INTO quality_profiles (
    id,
    name,
    scope,
    scope_id,
    archival_quality,
    allow_unknown_quality,
    atmos_preferred,
    dolby_vision_allowed,
    detected_hdr_allowed,
    prefer_remux,
    allow_bd_disk,
    allow_upgrades,
    created_at
)
VALUES
    (
        '4k',
        '4K',
        'system',
        NULL,
        '2160P',
        0,
        1,
        1,
        1,
        1,
        0,
        1,
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    ),
    (
        '1080p',
        '1080P',
        'system',
        NULL,
        '1080P',
        0,
        1,
        1,
        1,
        1,
        0,
        1,
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    );

INSERT OR IGNORE INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order)
VALUES
    ('4k', '2160P', 0),
    ('4k', '1080P', 1),
    ('4k', '720P', 2),
    ('1080p', '1080P', 0),
    ('1080p', '720P', 1);

DELETE FROM settings_values
WHERE setting_definition_id IN (
    SELECT id
    FROM settings_definitions
    WHERE key_name = 'quality.profiles'
);

DELETE FROM settings_definitions
WHERE key_name = 'quality.profiles';
