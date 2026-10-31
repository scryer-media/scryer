use async_trait::async_trait;
use scryer_application::{AppError, AppResult, SeedingProfileRepository};
use scryer_domain::{PostImportTracking, SeasonPackSeedMode, SeedGoalMetAction, SeedingProfile};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const SEEDING_PROFILE_COLUMNS: &str = "id, name, ratio, seed_time_minutes, season_pack_mode,
    season_pack_ratio, season_pack_seed_time_minutes, honor_tracker_minimums, goal_met_action,
    never_remove, minimum_seeders, post_import_tracking, created_at, updated_at";

const SEEDING_PROFILE_INSERT_SQL: &str = "INSERT INTO seeding_profiles (
    id, name, ratio, seed_time_minutes, season_pack_mode, season_pack_ratio,
    season_pack_seed_time_minutes, honor_tracker_minimums, goal_met_action, never_remove,
    minimum_seeders, post_import_tracking, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

const SEEDING_PROFILE_NAME_CONFLICT_SQL: &str =
    "SELECT id FROM seeding_profiles WHERE LOWER(name) = LOWER({}) AND id <> {}";

#[derive(Clone)]
pub struct SeedingProfileStore {
    datastore: StoreDatastore,
}

impl SeedingProfileStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl SeedingProfileRepository for SeedingProfileStore {
    async fn list(&self) -> AppResult<Vec<SeedingProfile>> {
        fetch_seeding_profiles(
            self.datastore.read_exec(),
            &format!("SELECT {SEEDING_PROFILE_COLUMNS} FROM seeding_profiles ORDER BY name ASC"),
            &[],
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<SeedingProfile>> {
        fetch_optional_seeding_profile(
            self.datastore.read_exec(),
            &format!("SELECT {SEEDING_PROFILE_COLUMNS} FROM seeding_profiles WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn create(&self, profile: SeedingProfile) -> AppResult<SeedingProfile> {
        let profile = profile.normalized();
        let args = seeding_profile_insert_args(&profile);
        SqlRuntime::run_in_transaction(&self.datastore, "create_seeding_profile", move |tx| {
            let profile = profile.clone();
            let args = args.clone();
            Box::pin(async move {
                ensure_unique_name(SqlExec::Tx(tx), &profile.name, &profile.id).await?;
                SqlRuntime::execute(SqlExec::Tx(tx), SEEDING_PROFILE_INSERT_SQL, &args).await?;
                Ok(profile)
            })
        })
        .await
    }

    async fn update(&self, profile: SeedingProfile) -> AppResult<SeedingProfile> {
        let profile = profile.normalized();
        let args = vec![
            SqlArg::Text(profile.name.clone()),
            SqlArg::OptF64(profile.ratio),
            SqlArg::OptI64(profile.seed_time_minutes),
            SqlArg::Text(profile.season_pack_mode.as_str().to_string()),
            SqlArg::OptF64(profile.season_pack_ratio),
            SqlArg::OptI64(profile.season_pack_seed_time_minutes),
            SqlArg::Bool(profile.honor_tracker_minimums),
            SqlArg::Text(profile.goal_met_action.as_str().to_string()),
            SqlArg::Bool(profile.never_remove),
            SqlArg::OptI64(profile.minimum_seeders.map(i64::from)),
            SqlArg::Text(profile.post_import_tracking.as_str().to_string()),
            SqlArg::Timestamp(profile.updated_at),
            SqlArg::Text(profile.id.clone()),
        ];
        SqlRuntime::run_in_transaction(&self.datastore, "update_seeding_profile", move |tx| {
            let profile = profile.clone();
            let args = args.clone();
            Box::pin(async move {
                ensure_unique_name(SqlExec::Tx(tx), &profile.name, &profile.id).await?;
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE seeding_profiles SET
                            name = {}, ratio = {}, seed_time_minutes = {}, season_pack_mode = {},
                            season_pack_ratio = {}, season_pack_seed_time_minutes = {},
                            honor_tracker_minimums = {}, goal_met_action = {}, never_remove = {},
                            minimum_seeders = {}, post_import_tracking = {}, updated_at = {}
                         WHERE id = {}",
                    &args,
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::not_found_reason(
                        crate::error_reasons::ACQUISITION_SEEDING_PROFILE_NOT_FOUND,
                        format!("seeding profile {}", profile.id),
                    ));
                }
                Ok(profile)
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_seeding_profile", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM seeding_profiles WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::not_found_reason(
                        crate::error_reasons::ACQUISITION_SEEDING_PROFILE_NOT_FOUND,
                        format!("seeding profile {id}"),
                    ));
                }
                Ok(())
            })
        })
        .await
    }
}

/// Names are unique case-insensitively. The unique index is the backstop; this
/// check turns the collision into a validation error operators can act on.
async fn ensure_unique_name(exec: SqlExec<'_, '_>, name: &str, id: &str) -> AppResult<()> {
    let conflict = SqlRuntime::fetch_optional(
        exec,
        SEEDING_PROFILE_NAME_CONFLICT_SQL,
        &[SqlArg::Text(name.to_string()), SqlArg::Text(id.to_string())],
    )
    .await?;
    if conflict.is_some() {
        return Err(AppError::validation_refused(
            crate::error_reasons::ACQUISITION_SEEDING_PROFILE_NAME_CONFLICT,
            format!("seeding profile name '{name}' is already in use"),
        ));
    }
    Ok(())
}

fn seeding_profile_insert_args(profile: &SeedingProfile) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(profile.id.clone()),
        SqlArg::Text(profile.name.clone()),
        SqlArg::OptF64(profile.ratio),
        SqlArg::OptI64(profile.seed_time_minutes),
        SqlArg::Text(profile.season_pack_mode.as_str().to_string()),
        SqlArg::OptF64(profile.season_pack_ratio),
        SqlArg::OptI64(profile.season_pack_seed_time_minutes),
        SqlArg::Bool(profile.honor_tracker_minimums),
        SqlArg::Text(profile.goal_met_action.as_str().to_string()),
        SqlArg::Bool(profile.never_remove),
        SqlArg::OptI64(profile.minimum_seeders.map(i64::from)),
        SqlArg::Text(profile.post_import_tracking.as_str().to_string()),
        SqlArg::Timestamp(profile.created_at),
        SqlArg::Timestamp(profile.updated_at),
    ]
}

async fn fetch_seeding_profiles(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<SeedingProfile>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_seeding_profile(&row))
        .collect()
}

async fn fetch_optional_seeding_profile(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<SeedingProfile>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row_to_seeding_profile(&row))
        .transpose()
}

fn row_to_seeding_profile(row: &SqlRow) -> AppResult<SeedingProfile> {
    let season_pack_mode = row.text("season_pack_mode")?;
    let goal_met_action = row.text("goal_met_action")?;
    Ok(SeedingProfile {
        id: row.text("id")?,
        name: row.text("name")?,
        ratio: row.opt_f64("ratio")?,
        seed_time_minutes: row.opt_i64("seed_time_minutes")?,
        season_pack_mode: SeasonPackSeedMode::parse(&season_pack_mode).ok_or_else(|| {
            AppError::Repository(format!(
                "unknown season pack seed mode '{season_pack_mode}'"
            ))
        })?,
        season_pack_ratio: row.opt_f64("season_pack_ratio")?,
        season_pack_seed_time_minutes: row.opt_i64("season_pack_seed_time_minutes")?,
        honor_tracker_minimums: row.bool("honor_tracker_minimums")?,
        goal_met_action: SeedGoalMetAction::parse(&goal_met_action).ok_or_else(|| {
            AppError::Repository(format!("unknown seed goal met action '{goal_met_action}'"))
        })?,
        never_remove: row.bool("never_remove")?,
        minimum_seeders: row
            .opt_i64("minimum_seeders")?
            .map(|value| value.clamp(i32::MIN as i64, i32::MAX as i64) as i32),
        // NULL only for a row that predates migration 0166; `Park` is the
        // shipped default and the direction that keeps managing the torrent.
        post_import_tracking: row
            .opt_text("post_import_tracking")?
            .as_deref()
            .and_then(PostImportTracking::parse)
            .unwrap_or_default(),
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn store() -> SeedingProfileStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        // The shipped migration also touches `indexers`, which this fixture does
        // not create; apply only the seeding-profile statements.
        for statement in include_str!("../../../scryer/src/db/migrations/0163_seeding_profiles.sql")
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty() && statement.contains("seeding_profiles"))
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("seeding profile schema should apply");
        }
        // 0166 also touches `download_submissions`, which this fixture does not
        // create; apply only its seeding-profile statement.
        for statement in include_str!(
            "../../../scryer/src/db/migrations/0166_seeding_profile_post_import_tracking.sql"
        )
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty() && statement.contains("seeding_profiles"))
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("post-import tracking schema should apply");
        }
        SeedingProfileStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn profile(id: &str, name: &str) -> SeedingProfile {
        let now = Utc::now();
        SeedingProfile {
            id: id.to_string(),
            name: name.to_string(),
            ratio: Some(1.5),
            seed_time_minutes: Some(4320),
            season_pack_mode: SeasonPackSeedMode::Inherit,
            season_pack_ratio: None,
            season_pack_seed_time_minutes: None,
            honor_tracker_minimums: true,
            goal_met_action: SeedGoalMetAction::RemoveEntry,
            never_remove: false,
            minimum_seeders: None,
            post_import_tracking: PostImportTracking::Park,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn seeding_profile_round_trips_through_crud() {
        let store = store().await;
        let created = store
            .create(profile("profile-1", "  Private tracker  "))
            .await
            .expect("profile should insert");
        assert_eq!(created.name, "Private tracker");

        let loaded = store
            .get_by_id("profile-1")
            .await
            .expect("profile should load")
            .expect("profile should exist");
        assert_eq!(loaded.ratio, Some(1.5));
        assert_eq!(loaded.seed_time_minutes, Some(4320));
        assert_eq!(loaded.season_pack_mode, SeasonPackSeedMode::Inherit);
        assert!(loaded.honor_tracker_minimums);
        assert_eq!(loaded.goal_met_action, SeedGoalMetAction::RemoveEntry);
        assert!(!loaded.never_remove);
        // The shipped column default: a profile that says nothing keeps Scryer
        // managing the torrent after import.
        assert_eq!(loaded.post_import_tracking, PostImportTracking::Park);

        let mut updated = loaded;
        updated.ratio = None;
        updated.season_pack_mode = SeasonPackSeedMode::Override;
        updated.season_pack_ratio = Some(2.0);
        updated.goal_met_action = SeedGoalMetAction::StopSeeding;
        updated.never_remove = true;
        updated.post_import_tracking = PostImportTracking::HandOff;
        updated.updated_at = Utc::now();
        store.update(updated).await.expect("profile should update");

        let reloaded = store
            .get_by_id("profile-1")
            .await
            .expect("profile should load")
            .expect("profile should exist");
        assert_eq!(reloaded.ratio, None);
        assert_eq!(reloaded.season_pack_mode, SeasonPackSeedMode::Override);
        assert_eq!(reloaded.season_pack_ratio, Some(2.0));
        assert_eq!(reloaded.goal_met_action, SeedGoalMetAction::StopSeeding);
        assert!(reloaded.never_remove);
        assert_eq!(reloaded.post_import_tracking, PostImportTracking::HandOff);

        assert_eq!(store.list().await.expect("list should load").len(), 1);
        store.delete("profile-1").await.expect("profile deletes");
        assert!(store.list().await.expect("list should load").is_empty());
        store
            .delete("profile-1")
            .await
            .expect_err("missing profile deletion should fail");
    }

    #[tokio::test]
    async fn seeding_profile_names_are_unique_case_insensitively() {
        let store = store().await;
        store
            .create(profile("profile-1", "Seedbox"))
            .await
            .expect("first profile should insert");
        let conflict = store
            .create(profile("profile-2", "seedbox"))
            .await
            .expect_err("duplicate name should be rejected");
        assert!(
            matches!(conflict, AppError::Validation(message) | AppError::ValidationRefused { message, .. } if message.contains("already in use"))
        );

        store
            .create(profile("profile-2", "Public"))
            .await
            .expect("distinct name should insert");
        let mut rename = store
            .get_by_id("profile-2")
            .await
            .expect("profile should load")
            .expect("profile should exist");
        rename.name = "SEEDBOX".to_string();
        let conflict = store
            .update(rename)
            .await
            .expect_err("rename onto an existing name should be rejected");
        assert!(
            matches!(conflict, AppError::Validation(message) | AppError::ValidationRefused { message, .. } if message.contains("already in use"))
        );
    }

    #[tokio::test]
    async fn season_pack_values_are_dropped_when_mode_inherits() {
        let store = store().await;
        let mut inherit = profile("profile-1", "Inherit");
        inherit.season_pack_mode = SeasonPackSeedMode::Inherit;
        inherit.season_pack_ratio = Some(3.0);
        inherit.season_pack_seed_time_minutes = Some(120);
        store.create(inherit).await.expect("profile should insert");

        let loaded = store
            .get_by_id("profile-1")
            .await
            .expect("profile should load")
            .expect("profile should exist");
        assert_eq!(loaded.season_pack_ratio, None);
        assert_eq!(loaded.season_pack_seed_time_minutes, None);
    }
}
