# Release Group Database Validation

You are validating and updating the scryer release group database before a release.
The database is at `crates/scryer-application/src/release_group_db.rs`.

## Your Task

1. **Research current group tiers** from these authoritative sources:
   - TRaSH Guides GitHub: `github.com/TRaSH-Guides/Guides` under `docs/json/radarr/cf/` and `docs/json/sonarr/cf/`
     - Files: `web-tier-01.json`, `web-tier-02.json`, `web-tier-03.json`
     - Files: `uhd-bluray-tier-01.json`, `uhd-bluray-tier-02.json`, `uhd-bluray-tier-03.json`
     - Files: `hd-bluray-tier-01.json`, `hd-bluray-tier-02.json`, `hd-bluray-tier-03.json`
     - Files: `remux-tier-01.json`, `remux-tier-02.json`, `remux-tier-03.json`
     - Files: `lq.json`, `lq-release-title.json`, `bad-dual-groups.json`
     - Anime: `anime-bd-tier-01.json` through `anime-bd-tier-08.json`, `anime-web-tier-01.json` through `anime-web-tier-06.json`, `anime-lq-groups.json`
   - Each JSON file has `"conditions"` with regex patterns containing group names separated by `|`
   - SeaDex (sneedex.moe) for anime group quality rankings

2. **Compare with current database** — identify:
   - Groups that TRaSH added since last update (add them)
   - Groups that TRaSH removed (remove them)
   - Groups that changed tiers (update them)
   - New banned/LQ groups (add them)

3. **Update the database file** — edit `crates/scryer-application/src/release_group_db.rs`:
   - Add new groups using the `group!()` macro
   - Remove groups no longer in TRaSH guides
   - Update tier assignments that changed
   - Keep existing structure and comments

4. **Run tests** to verify:
   ```bash
   cargo nextest run -p scryer-application release_group_db
   ```
   Fix any test failures from changed tiers.

## Tier Mapping

| TRaSH Tier | Our Tier | Meaning |
|------------|----------|---------|
| Tier 01 | Gold | Best quality groups for that source |
| Tier 02 | Silver | Great quality |
| Tier 03 | Bronze | Good quality |
| LQ / Bad Dual | Banned | Known problematic |

## Source Context Mapping

| TRaSH Category | Our SourceContext |
|----------------|-------------------|
| web-tier-* | Web |
| hd-bluray-tier-* | BluRay |
| uhd-bluray-tier-* | UhdBluRay |
| remux-tier-* | Remux |
| anime-bd-tier-01 through 03 | Anime (Gold/Silver/Bronze) |
| anime-lq-groups | Anime (Banned) |
| lq, bad-dual-groups | Any (Banned) |

## Anime Tier Mapping

TRaSH has 8 BD tiers and 6 WEB tiers for anime. We collapse these:
- BD Tier 01-02 / WEB Tier 01 → Gold (best fansub/encode groups)
- BD Tier 03-04 / WEB Tier 02-03 → Silver (good quality)
- BD Tier 05-06 / WEB Tier 04 → Bronze (acceptable)
- BD Tier 07-08 / WEB Tier 05-06 → not included (too granular, leave as unknown)
- Anime LQ → Banned

## Rules

- Do NOT add Co-Authored-By or otherwise put Claude's name on commits
- Keep the `group!()` macro format
- Preserve alphabetical ordering within each section
- Run the test suite after changes
- If a group appears in multiple source contexts at different tiers, add separate entries for each
- Case matters in group names — use the exact casing from TRaSH guides
