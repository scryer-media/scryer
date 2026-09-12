# Community rule packs

In **Settings → Rules**, choose a community pack and select which rules to enable.
Installation tracks every rule in the pack; an empty selection leaves them all
disabled. Existing custom rules and earlier template copies remain independent.
Installing a pack does not disable those older copies.

Installed packs show their version, update status, and rules. Enablement and
priority remain local settings. Pack names, descriptions, source, and facets are
maintained by the pack. Customizable packs offer **Copy to customize**:
saving creates a custom rule and disables its original so both do not score.
Packs such as SeaDex can disable customization. Their rules have no source
viewer, editor, or copy action; enablement, priority, and updates remain
available. **Test scoring** tests an installed rule against a selected library
title and release name without copying, saving, or exposing its source.
Uninstalling a pack removes its owned rules while preserving custom copies and
history.

Pack authors set `"customizable": false` in the pack manifest to disable
customization for all of its rules. Omission defaults to `true` for existing
packs. The catalog carries the setting for each release, and installation or
update persists it locally so it applies offline. Existing independent custom
copies remain independent; a later pack update does not remove them.

**Check for updates** previews added, changed, and removed template IDs before
applying an update. Existing rules keep their local identities, enablement, and
priority. New rules start disabled. Removed rules are retained but disabled;
if the same template returns later, it stays disabled until enabled explicitly.
A concurrent settings change invalidates an older preview.

**Auto-update** defaults off for each pack. It runs after the existing scheduled
plugin catalog refresh, independently of the plugin auto-update setting. Only
compatible stable releases on the installed major/minor are automatic. Minor
and major updates require the manual preview/apply flow. Refreshing the catalog
manually discovers updates without installing them.

Downloads use the existing signed catalog and bounded artifact verification.
Every source, including disabled rules, is validated as a user policy. The host
prepares the prospective evaluator before atomically saving an update; failed
validation or persistence leaves the previous rules active. Compilation is paid
during rule changes, not for each release evaluation. Changing only the
auto-update preference does not compile another evaluator.

Both system-settings and catalog-settings management permissions are required.
Pack ownership and preferences are included in logical backups. Packs remain
generic community plugins and cannot change quality-profile restrictions.
