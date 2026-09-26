impl AppUseCase {
    async fn require_title_permission(
        &self,
        actor: &User,
        title_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(actor, &title.library_id, permission)
            .await?;
        Ok(title)
    }
}
impl AppUseCase {
    pub(crate) async fn filter_title_ids_for_permission(
        &self,
        actor: &User,
        title_ids: &[String],
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<String>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let library_by_title = self
            .services
            .catalog
            .titles
            .get_by_ids(title_ids)
            .await?
            .into_iter()
            .map(|title| (title.id, title.library_id))
            .collect::<HashMap<_, _>>();

        let mut visible = Vec::with_capacity(title_ids.len());
        for title_id in title_ids {
            if let Some(library_id) = library_by_title.get(title_id)
                && allowed_library_ids.contains(library_id)
            {
                visible.push(title_id.clone());
            }
        }
        Ok(visible)
    }
}
impl AppUseCase {
    async fn require_collection_permission(
        &self,
        actor: &User,
        collection_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Collection> {
        let collection = self
            .services
            .catalog
            .shows
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("collection {collection_id}")))?;
        self.require_title_permission(actor, &collection.title_id, permission)
            .await?;
        Ok(collection)
    }
}
impl AppUseCase {
    async fn require_episode_permission(
        &self,
        actor: &User,
        episode_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Episode> {
        let episode = self
            .services
            .catalog
            .shows
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("episode {episode_id}")))?;
        self.require_title_permission(actor, &episode.title_id, permission)
            .await?;
        Ok(episode)
    }
}
