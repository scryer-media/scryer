-- Move the same-tier upgrade delta from a global setting into the profile.
--
-- `acquisition.same_tier_min_delta` was one number for the whole instance. It
-- now lives in each quality profile as `min_upgrade_score_delta`, inside the
-- `scoring_config` JSON, and an absent key means the default of 120.
--
-- Profiles are left without the key unless the instance had changed the old
-- setting. `convergence::profile_criteria_version` hashes the profile's
-- acceptance criteria, and an absent key leaves that hash where it was, so an
-- instance on the default re-searches nothing. An instance that had set its own
-- value keeps it: every profile gets that value, which moves each profile's
-- hash once and re-searches its scopes once. That is the cost of keeping the
-- upgrade behaviour the operator chose; dropping the value would silently
-- change it.
--
-- Only a whole, non-negative number is carried. A profile that already has the
-- key keeps its own value. The old setting row is left where it is; nothing
-- reads it any more.
WITH carried AS (
    SELECT CAST(json_extract(sv.value_json, '$') AS INTEGER) AS delta
      FROM settings_values sv
      JOIN settings_definitions sd ON sd.id = sv.setting_definition_id
     WHERE sd.key_name = 'acquisition.same_tier_min_delta'
       AND sv.scope = 'system'
       AND json_valid(sv.value_json)
       AND json_type(sv.value_json, '$') = 'integer'
     ORDER BY sv.updated_at DESC
     LIMIT 1
)
UPDATE quality_profiles
   SET scoring_config = json_set(
           scoring_config,
           '$.min_upgrade_score_delta',
           (SELECT delta FROM carried)
       )
 WHERE json_valid(scoring_config)
   AND json_type(scoring_config) = 'object'
   AND json_type(scoring_config, '$.min_upgrade_score_delta') IS NULL
   AND EXISTS (
           SELECT 1 FROM carried
            WHERE delta >= 0
              AND delta <> 120
       );
