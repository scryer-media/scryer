#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExternalImportLibraryPathsSelection {
    pub movie_paths: Vec<String>,
    pub series_paths: Vec<String>,
    pub anime_paths: Vec<String>,
}
fn normalize_external_import_root_folders(
    paths: Vec<String>,
) -> AppResult<Option<Vec<RootFolderEntry>>> {
    let normalized_paths = paths
        .into_iter()
        .filter_map(|path| normalize_optional_string(Some(path)))
        .collect::<Vec<_>>();

    if normalized_paths.is_empty() {
        return Ok(None);
    }

    let entries = normalized_paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| RootFolderEntry {
            path,
            is_default: index == 0,
        })
        .collect::<Vec<_>>();

    normalize_root_folders(entries).map(Some)
}
impl AppUseCase {
    pub async fn save_external_import_library_paths(
        &self,
        actor: &User,
        selection: ExternalImportLibraryPathsSelection,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut saved_any = false;
        for (facet, paths) in [
            (MediaFacet::Movie, selection.movie_paths),
            (MediaFacet::Series, selection.series_paths),
            (MediaFacet::Anime, selection.anime_paths),
        ] {
            let Some(root_folders) = normalize_external_import_root_folders(paths)? else {
                continue;
            };

            self.update_media_settings(
                actor,
                facet,
                UpdateMediaSettings {
                    library_path: None,
                    root_folders: Some(root_folders),
                    required_audio_languages: None,
                    folder_template: None,
                    season_folder_template: None,
                    rename_enabled: None,
                    rename_template: None,
                    rename_collision_policy: None,
                    rename_missing_metadata_policy: None,
                    filler_policy: None,
                    recap_policy: None,
                    monitor_specials: None,
                    inter_season_movies: None,
                    monitor_filler_movies: None,
                    nfo_write_on_import: None,
                    plexmatch_write_on_import: None,
                    import_mode: None,
                },
            )
            .await?;
            saved_any = true;
        }

        if !saved_any {
            return Ok(false);
        }

        Ok(true)
    }
}
