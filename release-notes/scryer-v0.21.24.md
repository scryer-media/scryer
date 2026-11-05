# Scryer 0.21.24 release notes

These changes are relative to v0.21.23.

## Highlights

- Support Prowlarr 3.x alongside Prowlarr 2.x, without requiring the 0.22.0 release.
- Fix global search for users who can manage a library but cannot manage catalog-wide settings, and restrict root-folder listings to libraries the user can see.
- Improve verified season-pack imports and manual import previews, including ordinal filenames in anime packs.
- Cancel acquisition searches from the search-progress toast or the Jobs view.
- Add `{dynamic_range}`, `{revision}`, and `{is_3d}` rename tokens, with editor previews.
- Expand Russian translations across the interface, including setup, settings, calendar, job descriptions, and provider field labels.

## Prowlarr compatibility

- Fix connection tests for saved Prowlarr-managed indexers without changing their synchronized URLs.
- Prowlarr 2.x and 3.x use the same supported management API for connection tests, indexer sync, app profiles, and child capabilities. Existing connection settings continue to work.
- A newer major version is allowed through the known API generation with a warning that it is newer than the tested releases, rather than being rejected solely for its major version.
- Versions older than 2.0 are refused with a translated unsupported-version explanation.
- Display the reported Prowlarr version reliably in the external-import wizard.

## Import and parsing fixes

- Use stored release evidence to distinguish the episode that initiated a grab from other verified members of its season pack. Manual previews, automatic imports, and completion checks share the same membership rules.
- Recognize ordinal filenames only when the title and single-season episode membership can be established. Preserve ordinary episode numbers and ranges, and keep ambiguous files, extras, and opening or ending material for review.
- Preserve duplicate, monitoring, quality, and season-scope checks when importing verified pack members.
- Fix compound release-group recognition without absorbing preceding release tags into the group name.

## Localization and metadata

- Add Russian plural handling and calendar localization, and replace additional English fallback text and accessibility labels with translations.
- Translate known built-in job descriptions and provider field labels while preserving custom text and protocol values.
- Retain alternate-title country information alongside language metadata.

## API and upgrade notes

- Add `TitlePayload.taggedAliases`, with `name`, `language`, and optional `country` fields. The existing string-only `aliases` field remains available.
- Plugin alias metadata gains optional country information; aliases without it remain compatible.
- This release adds no database migrations and requires no Prowlarr connection-setting changes.
