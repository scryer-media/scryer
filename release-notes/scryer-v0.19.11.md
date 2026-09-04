# scryer-v0.19.11

AI generated release notes

## Highlights

- Acquisition is more reliable. Scryer now keeps a grabbed release claimed until it is actually resolved, avoids re-grabbing the same release while it is still queued, post-processing, or temporarily unreadable from the download client, and reopens the scope correctly after an operator ignores or removes that grab.
- Manual Import now handles verified multi-season packs correctly, including season packs whose names lose range hyphens, and import summaries now report every hold reason instead of only the last one.
- Administrators and catalog admins now keep the expected access to libraries added after their accounts were provisioned, and the web UI now shows the matching management controls.
- Movie scans now choose the better primary file by quality instead of keeping an older, worse primary when multiple primary candidates are found.
- Download tracking now skips repeated observations for client jobs whose bindings have already ended, reducing noisy ended-job handling.

## Included fixes

- Improved season-run parsing for names such as `S01-S02-S03`, `S01~S02~S03`, `S01 S02`, and `S01.S02`, which prevents valid multi-season pack members from being held as outside the declared seasons.
- Tightened acquisition admission and pending-release handling so equal-score churn and transient client visibility gaps do not cause duplicate grabs of the same release.
- Aligned backend permission checks and frontend permission helpers so administrator behavior is consistent across API responses and the web app.