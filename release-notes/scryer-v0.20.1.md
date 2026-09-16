# Scryer 0.20.1 release notes

These notes cover what's changed since **0.20.0**.

## Highlights

- **Libraries with more than one root folder behave correctly.** A library scan used to put every title it found on the library's default root, even when the files lived under a second root. Preview Rename then proposed moving those titles across roots, and applying it moved only the video file, leaving the folder, its `.nfo` and its external subtitles behind. This closes issue #224.
  - Scans now put a new title on the root folder it was found under, and correct existing titles whose recorded root doesn't match where their folder actually is.
  - Rename always stays inside the root that holds the title's folder. Moving a title to another root is the job of a move, not a rename.
  - Rename now moves external subtitles and `.nfo` files along with their video file and removes season and title folders the rename leaves empty. It never removes a root folder.

- **Series that release groups number by TVDB's alternate or DVD order now match.** Some groups number a series by its alternate or DVD episode order while Scryer's catalog follows the official order, so releases landed on the wrong episodes or never matched. Scryer now reads those orders from TVDB and can translate between them.
  - A new **Release numbering** setting on each series chooses between **Auto**, **Official order**, **Alternate order** and **DVD order**.
  - **Auto** uses the alternate reading only when a release clearly calls for it, for example when the episode number it names exists only in that order. Otherwise the release is read as numbered.
  - Picking **Alternate order** or **DVD order** pins the series to that order. Only pinned series are also searched with alternate episode numbers, which keeps indexer usage down.
  - Changing the setting rebuilds the series' numbering in the background right away.
  - Anime community numbering works as before and keeps priority under **Auto**.

- **One file can be manually imported as several episodes.** A two-episode file such as `S01E27E28` could only be assigned to one episode, so the other stayed wanted and was downloaded again. Manual import now has a **Several episodes in this file** option, suggests the whole set when the file name names more than one episode, and has **Apply suggestions to all** for a batch of files. All episodes for one file must be from the same season.

- **Grabs no longer get stuck behind a download that disappeared.** Since 0.19.14, Scryer holds a new grab while an earlier download for the same episode or movie is still being tracked. If the download client no longer had that earlier download, the new grab could wait indefinitely.
  - Scryer now checks the download client for that missing download and releases the hold.
  - While a grab is held, you see a warning explaining why instead of an error.

- **See everything Scryer knows about a media file.** Every media file row now shows its audio tracks and has an **Info** button that opens the complete details: file, video, HDR and Dolby Vision, audio, subtitle and caption tracks, chapters, attachments, the release it came from, and the media analysis results.

## Included fixes

- **Moves (experimental):**
  - An existing `.scryer-partial` file at a move's destination is no longer deleted unless it's this move's own unfinished copy. Otherwise the copy fails and both files are kept.
  - When a move finds an identical copy already at the destination, it re-checks the destination file against the recorded checksum before removing the source.
  - A move interrupted by a restart is picked up again from Activity instead of being marked failed during startup.
- **Maintenance rules (experimental):**
  - Watch history from a media server connection that has been turned off no longer counts toward a rule, at either the title or episode level.
  - A connection that's turned back on is treated as not yet synced until its next successful sync.
  - Maintenance searches that were running when Scryer restarted resume instead of failing.
- **Library and rename:**
  - A title being scanned and looked up at the same time no longer ends up with no metadata and its files skipped.
  - Titles whose files are outside every configured root folder are skipped by rename, with a reason, instead of being moved.
  - An external subtitle or `.nfo` file that can't be moved back after a failed rename is reported at the location it was moved to, with the reason.
- **Search and downloads:**
  - Downloads you start from interactive search in the browser now count toward the indexer's grab limits.
  - Archive extraction never writes through a symbolic link found in the output folder.
  - A failure while reading anime numbering data no longer leaves an incomplete episode matcher in use.
  - **Activity → History now lists ignored downloads.** Choosing Ignore on a queue item records a history entry, but the page's default view left it out, so the only lasting record of a download that left the queue without importing was invisible. Ignored downloads now appear with the other events and have their own filter.
  - Operators that need different indexer backoff periods can now set `SCRYER_INDEXER_BACKOFF_LADDER_SECS` to a comma-separated, ascending list of seconds, for example `15,30,45,90,180`. Unset or unusable values keep the shipped 5/10/15/30/60-minute backoff, so the existing default behavior is unchanged.
  - `SCRYER_RSS_TARGET_INTERVAL_SECS` now also sets how often the RSS sync worker wakes, so a cadence shorter than a minute takes effect instead of being rounded up to the worker's next wake. A cadence of a minute or longer, and leaving the variable unset, keep the existing once-a-minute wake.
  - A torrent held for seeding after its import no longer disappears from the queue a second later and gets ignored, then re-added as if Scryer had never started it.
  - Deleting a download from History while the torrent is still in the client no longer briefly re-adds it to Activity as a download Scryer never started.
  - When an indexer is pinned to one download client, a grab from that indexer is now sent to it even while its queue can't be read, so the client's own error is recorded instead of the release quietly staying pending.
  - When a season pack and the individual episodes it contains score the same, the season pack is grabbed instead of one download per episode. An episode release that scores higher than the pack still wins.
- **Login settings:**
  - If you set the login lifetime with the `SCRYER_JWT_ACCESS_TTL_SECONDS` environment variable, it's honored again. It's rounded to the nearest whole day, from 1 to 365. A value you've set in **Settings → Security** takes precedence.
  - The Security page's form login panel is now called **Login settings**. **Minimum password length** and **Login valid for** sit side by side, with the lifetime entered in days.
- **Rules:**
  - The scoring rule template gallery has been trimmed. The Japanese audio, English audio, multi-audio, suspiciously small size and password-protected templates are gone. The release group, size and codec templates are each combined into one. A new **No .exe torrents** template penalizes torrents whose title names an executable file.
  - The AI rule prompt dialog is wider and shows a shortened preview with a copy button; copying still copies the full prompt.
  - The import buttons for Sonarr and Radarr custom formats show those apps' logos.
  - Rule pack auto-update checkboxes are easier to click.
- **Proxies:** creating a proxy can be cancelled at any step, including while importing a WireGuard configuration.
- **Appearance:** colored borders show their intended color throughout the app. Many used to show the default border color instead.

## Upgrading

Scryer runs one small database update at startup that adds tracking for where each series' episode numbering comes from. Existing anime numbering is kept as-is.

Series pick up alternate and DVD episode orders when their metadata is refreshed individually, or immediately when you change their **Release numbering** setting. The routine bulk metadata refresh keeps numbering already found but doesn't look for new orders.

No configuration changes are required.
