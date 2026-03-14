# Release Group Database Validation

You are validating and updating the scryer release group database before a release.
The database is at `crates/scryer-application/src/release_group_db.rs`.

## Data Source

A `<trash-guides-json>` block is appended below containing all relevant TRaSH Guides
custom format files pre-scraped from GitHub. Parse this JSON directly.
Do NOT attempt to fetch anything from the web.

The JSON structure is:
```json
{
  "scraped_at": "2026-03-14T...",
  "radarr": { "web-tier-01": { CF JSON ... }, "web-tier-02": { ... }, ... },
  "sonarr": { "web-tier-01": { CF JSON ... }, "web-tier-02": { ... }, ... }
}
```

Each CF JSON object has a `"specifications"` array. Entries with
`"implementation": "ReleaseGroupSpecification"` contain group names in the `"name"` field.
Entries with `"implementation": "ReleaseTitleSpecification"` are title-based patterns (for LQ).

Group names appear in both radarr and sonarr — deduplicate across both.

## Your Task

1. **Extract group names** from the pre-scraped JSON for each tier/source category
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
