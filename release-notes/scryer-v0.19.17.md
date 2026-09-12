# Scryer 0.19.17 release notes

## Highlights

- Jellyfin 12 servers work again. Jellyfin 12 stopped accepting the `X-Emby-Token` header Scryer authenticated with, so every call Scryer made to a Jellyfin 12 server came back unauthorized: connecting a media server, listing its users, linking accounts and scanning the library all failed, typically showing `unauthorized: Jellyfin admin token cannot list API keys` when adding the connection. This closes issue #211. Scryer now authenticates with the `Authorization: MediaBrowser` header that Jellyfin 12 requires.

## Included fixes

- Connecting a Jellyfin media server works with either an admin login or a pasted API key. The admin login path can once again read and create Scryer's API key on the server.
- Jellyfin user lists load, so Jellyfin users can be added and given access to requests again.
- Jellyfin account links and logins verify against the server again.
- Jellyfin library scans authenticate again, so Scryer can see what is already in the library and stops treating owned titles as missing.
- Jellyfin 10.x servers and Emby servers are unaffected: Scryer sends both the new and the previous credential on every request, so no server generation needs a different setting.

## Upgrading

No migration. No configuration changes.

If you turned on **Enable legacy authorization** in your Jellyfin server's settings to work around this, you no longer need it and can turn it off again.
