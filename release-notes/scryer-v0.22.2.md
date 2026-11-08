# Scryer 0.22.2 release notes

These changes are relative to v0.22.1.

## Highlights

- Russian is available as a metadata language.
- Searches by TMDB, IMDb or TVDB id no longer return unrelated releases from indexers that pad id lookups with other results, such as DS.
- Disabled built-in indexer plugins can be enabled again.

## Indexers

- A release returned by an id-based search is kept only when its name matches the title or one of its aliases, or when the indexer tags it with the title's id. Releases the indexer returned for a different title are dropped.
- Direct Newznab and Torznab searches that have no routed categories now send the standard defaults: 2000 for movies and 5000 for series and anime. Previously these searches went out without categories, and some indexers answered with results from every category.
- A built-in indexer plugin that was disabled can be re-enabled from the plugin settings.

## Metadata

- Add Russian (`rus`) to the supported metadata languages.
- The message shown for an unsupported metadata language now lists Dutch, Czech and Russian with the other supported languages.
