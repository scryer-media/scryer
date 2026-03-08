use super::*;

impl AppUseCase {
    pub async fn system_health(&self, actor: &User) -> AppResult<SystemHealth> {
        require(actor, &Entitlement::ManageConfig)?;

        let titles = self.services.titles.list(None, None).await?;
        let users = self.services.users.list_all().await?;
        let recent_events = self.services.events.list(None, 12, 0).await?;

        let mut titles_movie = 0usize;
        let mut titles_tv = 0usize;
        let mut titles_anime = 0usize;
        let mut titles_other = 0usize;
        let mut monitored_titles = 0usize;
        let mut recent_event_preview = Vec::with_capacity(std::cmp::min(3, recent_events.len()));

        for title in &titles {
            if title.monitored {
                monitored_titles += 1;
            }

            match title.facet {
                MediaFacet::Movie => titles_movie += 1,
                MediaFacet::Tv => titles_tv += 1,
                MediaFacet::Anime => titles_anime += 1,
                MediaFacet::Other => titles_other += 1,
            }
        }

        for event in recent_events.iter().take(3) {
            recent_event_preview.push(event.message.clone());
        }

        let db_migration_version = self.services.system_info.current_migration_version().await.ok().flatten();
        let db_pending_migrations = self.services.system_info.pending_migration_count().await.unwrap_or(0);
        let smg_cert_expires_at = self.services.system_info.smg_cert_expires_at().await.ok().flatten();
        let smg_cert_days_remaining = smg_cert_expires_at.as_deref().and_then(|expires_str| {
            chrono::DateTime::parse_from_rfc3339(expires_str)
                .ok()
                .map(|expires| (expires.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days())
        });

        let indexer_stats = self.services.indexer_stats.all_stats();

        Ok(SystemHealth {
            service_ready: true,
            db_path: self.services.db_path.clone(),
            total_titles: titles.len(),
            monitored_titles,
            total_users: users.len(),
            titles_movie,
            titles_tv,
            titles_anime,
            titles_other,
            recent_events: recent_events.len(),
            recent_event_preview,
            db_migration_version,
            db_pending_migrations,
            smg_cert_expires_at,
            smg_cert_days_remaining,
            indexer_stats,
        })
    }

    pub async fn disk_space(&self, actor: &User) -> AppResult<Vec<DiskSpaceInfo>> {
        require(actor, &Entitlement::ViewCatalog)?;

        // Collect unique root folder paths from all facet handlers
        let path_keys = [
            ("series.path", "/media/series", "Series"),
            ("anime.path", "/media/anime", "Anime"),
            ("movies.path", "/media/movies", "Movies"),
        ];

        let mut seen_paths = std::collections::HashSet::new();
        let mut results = Vec::new();

        for (key, default, label) in &path_keys {
            let path = self
                .read_setting_string_value_for_scope(
                    SETTINGS_SCOPE_MEDIA,
                    key,
                    None,
                )
                .await?
                .unwrap_or_else(|| default.to_string());

            if !seen_paths.insert(path.clone()) {
                continue; // skip duplicate mount points
            }

            match nix::sys::statvfs::statvfs(path.as_str()) {
                Ok(stat) => {
                    let block_size = stat.block_size() as u64;
                    let total = stat.blocks() as u64 * block_size;
                    let free = stat.blocks_available() as u64 * block_size;
                    let used = total.saturating_sub(free);
                    results.push(DiskSpaceInfo {
                        path,
                        label: label.to_string(),
                        total_bytes: total,
                        free_bytes: free,
                        used_bytes: used,
                    });
                }
                Err(err) => {
                    tracing::warn!(path = path.as_str(), error = %err, "failed to query disk space");
                }
            }
        }

        Ok(results)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<HistoryEvent> {
        self.services.event_broadcast.subscribe()
    }

    pub async fn ensure_default_admin(&self, username: &str, password: &str) -> AppResult<User> {
        if username.trim().is_empty() {
            return Err(AppError::Validation("admin username is required".into()));
        }
        if password.trim().is_empty() {
            return Err(AppError::Validation("admin password is required".into()));
        }
        let desired_entitlements = User::all_entitlements();

        if let Some(found) = self.services.users.get_by_username(username).await? {
            if !found.has_all_entitlements() {
                let user = self
                    .services
                    .users
                    .update_entitlements(&found.id, desired_entitlements)
                    .await?;
                return Ok(user);
            }
            return Ok(found);
        }

        let user = User {
            id: Id::new().0,
            username: username.to_string(),
            password_hash: Some(self.hash_password(password)?),
            entitlements: desired_entitlements,
        };

        self.services.users.create(user.clone()).await?;
        Ok(user)
    }

    pub async fn find_or_create_default_user(&self) -> AppResult<User> {
        self.ensure_default_admin("admin", "admin").await
    }

    pub async fn list_users(&self, actor: &User) -> AppResult<Vec<User>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services.users.list_all().await
    }

    pub async fn get_user(&self, actor: &User, user_id: &str) -> AppResult<Option<User>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services.users.get_by_id(user_id).await
    }

    pub async fn create_user(
        &self,
        actor: &User,
        username: String,
        password: String,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        require(actor, &Entitlement::ManageConfig)?;

        let username = username.trim().to_string();
        if username.is_empty() {
            return Err(AppError::Validation("username is required".to_string()));
        }
        let password_hash = self.hash_password(&password)?;

        if self
            .services
            .users
            .get_by_username(&username)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "user {} already exists",
                username
            )));
        }

        let user = User {
            id: Id::new().0,
            username: username.clone(),
            password_hash: Some(password_hash),
            entitlements,
        };

        let user = self.services.users.create(user).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("user created: {}", username),
            )
            .await?;

        Ok(user)
    }

    /// Set a user's password without actor checks. Used only for first-run bootstrap.
    pub async fn bootstrap_user_password(&self, user_id: &str, password: &str) -> AppResult<User> {
        let password_hash = self.hash_password(password)?;
        self.services
            .users
            .update_password_hash(user_id, password_hash)
            .await
    }

    pub async fn set_user_password(
        &self,
        actor: &User,
        user_id: &str,
        password: String,
        current_password: Option<String>,
    ) -> AppResult<User> {
        if password.trim().is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let existing = self
            .services
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if existing.id == actor.id {
            // Self-change: verify current password.
            let current = current_password
                .ok_or_else(|| AppError::Validation("current password is required".into()))?;
            let hash = existing
                .password_hash
                .as_deref()
                .ok_or_else(|| AppError::Validation("account has no password set".into()))?;
            if !self.validate_password(&current, hash)? {
                return Err(AppError::Unauthorized("current password is incorrect".into()));
            }
        } else {
            require(actor, &Entitlement::ManageConfig)?;
        }

        let password_hash = self.hash_password(&password)?;
        let user = self
            .services
            .users
            .update_password_hash(user_id, password_hash)
            .await?;

        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("user password updated: {}", user.username),
            )
            .await?;

        Ok(user)
    }

    pub async fn set_user_entitlements(
        &self,
        actor: &User,
        user_id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        require(actor, &Entitlement::ManageConfig)?;

        if entitlements.is_empty() {
            return Err(AppError::Validation(
                "at least one entitlement is required".into(),
            ));
        }

        let existing = self
            .services
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if existing.id == actor.id {
            return Err(AppError::Validation(
                "cannot modify own entitlements".into(),
            ));
        }

        let user = self
            .services
            .users
            .update_entitlements(user_id, entitlements)
            .await?;

        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("user entitlements updated: {}", user.username),
            )
            .await?;

        Ok(user)
    }

    pub async fn delete_user(&self, actor: &User, user_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;

        let user = self
            .services
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if user.id == actor.id {
            return Err(AppError::Validation("cannot delete current user".into()));
        }

        self.services.users.delete(user_id).await?;
        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("user deleted: {}", user.username),
            )
            .await?;

        Ok(())
    }
}
