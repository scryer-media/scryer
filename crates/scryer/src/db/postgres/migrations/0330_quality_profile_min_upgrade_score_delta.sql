-- Move the same-tier upgrade delta from a global setting into the profile.
--
-- See the SQLite migration of the same number for the reasoning: profiles stay
-- without the key (the default of 120) unless the instance had changed
-- `acquisition.same_tier_min_delta`, in which case every profile that has no
-- value of its own receives that one. The old setting row is left in place.
WITH carried AS (
    SELECT (sv.value_json #>> '{}')::integer AS delta
      FROM settings_values sv
      JOIN settings_definitions sd ON sd.id = sv.setting_definition_id
     WHERE sd.key_name = 'acquisition.same_tier_min_delta'
       AND sv.scope = 'system'
       AND jsonb_typeof(sv.value_json) = 'number'
       AND (sv.value_json #>> '{}') ~ '^[0-9]{1,9}$'
     ORDER BY sv.updated_at DESC
     LIMIT 1
)
UPDATE quality_profiles
   SET scoring_config = jsonb_set(
           quality_profiles.scoring_config,
           '{min_upgrade_score_delta}',
           to_jsonb(carried.delta)
       )
  FROM carried
 WHERE carried.delta <> 120
   AND jsonb_typeof(quality_profiles.scoring_config) = 'object'
   AND NOT jsonb_exists(quality_profiles.scoring_config, 'min_upgrade_score_delta');
