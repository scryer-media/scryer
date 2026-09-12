# Scryer 0.19.17 release notes

## Highlights

- Jellyfin 12 servers work again. Jellyfin 12 stopped accepting the `X-Emby-Token` header Scryer authenticated with, so every call Scryer made to a Jellyfin 12 server came back unauthorized: connecting a media server, listing its users, linking accounts and scanning the library all failed, typically showing `unauthorized: Jellyfin admin token cannot list API keys` when adding the connection. This closes issue #211. Scryer now authenticates with the `Authorization: MediaBrowser` header that Jellyfin 12 requires.

- Library scans recover titles whose folder moved outside Scryer. When files were relocated to another root folder by a different tool (a root-folder move, a drive migration, a root split), every affected title surfaced as a pending import with the reason "title already owns another folder" on every scan, and the scan dropped the title's catalog files, which could leave monitored titles looking missing. Scryer now notices that the recorded folder is gone while its root is still populated, re-points the title at the folder the files actually live in, and keeps the catalog files. Affected libraries heal on their next full scan; the pending imports clear themselves.

- Deleting a title together with its files works in every library. The delete guard compared the title's folder against the root folders of the facet's default library only, so a title in a second library of the same kind — a separate Anime or Movies library with its own root — was always refused with "outside the configured root folders". The guard now checks the roots of the library the title actually belongs to.

## Included fixes

- A title whose recorded folder no longer exists is re-pointed at the folder a scan finds its files in, for both movies and series. This only happens when the old folder's parent directory is still populated; a root that is missing or empty is treated as an unmounted drive and left alone.
- Scans no longer remove a title's catalog files over a folder-ownership conflict unless the folder the title owns is confirmed to still exist. A conflict against a vanished or unreachable folder keeps the files, so the title is not treated as missing and re-downloaded.
- Connecting a Jellyfin media server works with either an admin login or a pasted API key. The admin login path can once again read and create Scryer's API key on the server.
- Jellyfin user lists load, so Jellyfin users can be added and given access to requests again.
- Jellyfin account links and logins verify against the server again.
- Jellyfin library scans authenticate again, so Scryer can see what is already in the library and stops treating owned titles as missing.
- Jellyfin 10.x servers keep working: Scryer sends both the new and the previous credential on every Jellyfin request, so no Jellyfin generation needs a different setting.
- Emby servers are unaffected. Emby keeps its own credential path and its own request code, separate from Jellyfin, so no Jellyfin credential is ever sent to an Emby server.
- TRaSH Guides data is refreshed to 828 active release-group rules. The release group `HHWEB` is no longer part of WEB Tier 03 for movies or series, following the upstream guide change; it is not scored as another tier instead. Every score set keeps its existing minimum, maximum and veto values.

## Upgrading

No migration. No configuration changes.

If you turned on **Enable legacy authorization** in your Jellyfin server's settings to work around this, you no longer need it and can turn it off again.
