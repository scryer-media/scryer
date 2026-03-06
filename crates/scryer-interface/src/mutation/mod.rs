mod collections;
mod config;
mod downloads;
mod library;
mod plugins;
mod rules;
mod settings;
mod titles;
mod users;
mod wanted;

use async_graphql::MergedObject;

#[derive(MergedObject, Default)]
pub struct MutationRoot(
    titles::TitleMutations,
    collections::CollectionMutations,
    downloads::DownloadMutations,
    config::ConfigMutations,
    settings::SettingsMutations,
    users::UserMutations,
    library::LibraryMutations,
    wanted::WantedMutations,
    rules::RulesMutations,
    plugins::PluginMutations,
);
