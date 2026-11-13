# Permissions by area

A key carries the app permissions of the account that created it. It never
carries account-management rights: creating, listing or revoking keys, and
changing one's own password or MFA, need an interactive sign-in.

"Config" in the last column means the call also needs MFA step-up when form
login is on and **Require MFA step-up for configuration changes** is enabled.
An API key can never satisfy that check, so the call fails with
`MFA_STEP_UP_REQUIRED` (or `TOTP_ENROLLMENT_REQUIRED` if the account has no
authenticator). Calls not marked config only check the permission.

| Area | Operations | Permission | Config |
|---|---|---|---|
| Identity check | `me` | any key | no |
| Rule sets | `validateRuleSet`, `testRuleSet`, `createRuleSet`, `updateRuleSet`, `toggleRuleSet`, `translateArrCustomFormats` | `MANAGE_CATALOG_SETTINGS` | yes |
| Rule sets (read) | `ruleSets`, `ruleSet` | any key | no |
| Libraries: create | `createLibrary` | `MANAGE_CATALOG_SETTINGS` | yes |
| Libraries: edit, scan | `updateLibrary`, `scanLibrary` | manage-library grant on that library | no |
| Quality profiles | `saveQualityProfileSettings` | `MANAGE_CATALOG_SETTINGS` | yes |
| Delay profiles | `upsertDelayProfile` | `MANAGE_CATALOG_SETTINGS` | yes |
| Indexer and client routing | `updateIndexerRouting`, `updateDownloadClientRouting` | `MANAGE_CATALOG_SETTINGS` | yes |
| Tag definitions | `createTitleTagDefinition` | `MANAGE_CATALOG_SETTINGS` | yes |
| Media settings | `updateMediaSettings` | `MANAGE_CATALOG_SETTINGS` | no |
| Download client config | `updateDownloadClientConfig` | `MANAGE_SYSTEM_SETTINGS` | yes |
| Arr import: connection test, warm-up, preview | `validateExternalImportConnection` (Sonarr, Radarr), `startExternalImportArrSourceWarmup`, `previewExternalImport` | `MANAGE_CATALOG_SETTINGS` | no |
| Arr import: Prowlarr | `validateExternalImportConnection` (Prowlarr), `startExternalImportProwlarrWarmup` | `MANAGE_SYSTEM_SETTINGS` (the warm-up also needs `MANAGE_CATALOG_SETTINGS`) | no |
| Arr import: create indexers and clients | `startExternalImportSources`, `externalImportSourceStatus`, `retryExternalImportSources`, `skipExternalImportSource` | `MANAGE_SYSTEM_SETTINGS` | yes |
| Notifications | `createNotificationChannel` | `MANAGE_SYSTEM_SETTINGS` | yes |
| Arr import: finalize | `finalizeExternalImport` | `MANAGE_CATALOG_SETTINGS` | no |
| Users, permissions, lists | user and list operations | `MANAGE_USERS`, `MANAGE_PERMISSIONS`, `MANAGE_LISTS` | varies |

Library-scoped reads (titles, scans) also respect per-library grants. An
account that cannot see a library will not see its titles through the key
either.

When a call fails with `UNAUTHORIZED`, name the missing permission from this
table and stop. Do not look for another account or key.
