# scryer-v0.19.10

AI generated release notes

Scryer v0.19.10 improves monitoring controls, request presentation, and episode file cleanup, while also tightening OAuth, plugin, downloader, backup, and local-network behavior.

## Highlights
- Added advanced monitoring selection when adding or requesting media, including richer season-level and series/movie monitoring controls.
- Added multi-episode file deletion with checkbox selection, with larger delete work moved into background jobs for smoother library management.
- Request cards now display background art, and request actions stay usable while refreshes are in progress.
- Settings forms for indexers, download clients, and media servers now keep addresses exactly as typed.

## Fixes and polish
- Improved the season monitor picker with better column filling, clearer named season labels, specials sorted last, steadier button sizing, and aligned selection controls.
- Preserved request poster ratios and reduced duplicate refreshes after request actions on the dashboard and requests views.
- Improved Jellyfin plugin OAuth handling, including more reliable client identification and approval fingerprint handling.
- Restored deprecated plugin lifecycle status rendering in setup and settings views.
- Made download cleanup more reliable across supported clients, including better fallback behavior when queue/history hints are wrong and better handling of Weaver GraphQL delete errors.
- Trusted operator-configured origins on link-local addresses for better local-network deployments.
- Included monitor selections in backups so restored data keeps more of the original monitoring intent.

## Upgrade notes
- This release includes schema and database migration updates for monitor selections, request background art, and OAuth client kind storage.