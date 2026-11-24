# Maintainerr import codes

Codes come back as `extensions.reason` on the start or status call, as the
run's `reason`, as a run fact `code`, or as an item's `reason` or fact `code`.
Facts never stop the run. Branch on the code, never on the message.

## Run reasons (the run stopped)

| reason | Meaning, fix |
|---|---|
| `EXTERNAL_IMPORT_SOURCE_UNREACHABLE` | Refused, timed out or name did not resolve, from where Scryer runs. Fix `baseUrl`. |
| `EXTERNAL_IMPORT_SOURCE_UNEXPECTED_RESPONSE` | Error status or wrong JSON; often `baseUrl` points at a proxy page or another app. |
| `EXTERNAL_IMPORT_SOURCE_ENDPOINT_MISSING` | Maintainerr lacks an endpoint; usually too old. |
| `EXTERNAL_IMPORT_SOURCE_RESPONSE_TOO_LARGE`, `MAINTAINERR_IMPORT_JSON_TOO_LARGE` | Maintainerr answered with more data than Scryer reads. |
| `EXTERNAL_IMPORT_SOURCE_URL_INVALID`, `EXTERNAL_IMPORT_SOURCE_URL_QUERY_OR_FRAGMENT` | Bad URL, or a `?…`/`#…` part. |
| `MAINTAINERR_IMPORT_GROUPS_JSON_INVALID` | The rule groups could not be read. |
| `MAINTAINERR_IMPORT_CONSTANTS_JSON_INVALID` | Maintainerr's property catalogue could not be read. |
| `MAINTAINERR_IMPORT_PROFILES_JSON_INVALID` | A Sonarr or Radarr profile list from Maintainerr could not be read. |
| `MAINTAINERR_IMPORT_GROUPS_TOO_MANY`, `RULE_IMPORT_TOO_MANY_ITEMS` | More rule groups than one import accepts. |
| `RULE_IMPORT_FETCH_FAILED` | Maintainerr could not be read; run `message` says why. |
| `RULE_IMPORT_PLAN_FAILED` | Scryer could not load its own quality profiles or user names; nothing saved. Check Scryer's logs, then start again. |
| `RULE_IMPORT_INTERRUPTED` | Scryer stopped mid-run. Start again: saved groups are skipped. |

Start refusals: `RULE_IMPORT_INVALID_REQUEST` (empty `requestKey` or bad
field), `RULE_IMPORT_REQUEST_CHANGED` (key reused with another URL; use a new
key), `RULE_IMPORT_QUEUE_FULL` (wait for a run to settle). Status:
`RULE_IMPORT_NOT_OWNED` (unknown run, or another account's).

## Run facts (the run went on)

| code | Meaning |
|---|---|
| `RULE_IMPORT_PROFILES_UNAVAILABLE` | Maintainerr could not list one Sonarr or Radarr server's quality profiles; groups changing to one of its profiles fail. |
| `RULE_IMPORT_TOO_MANY_PROFILE_SOURCES` | Groups name more Sonarr or Radarr servers than one import reads profiles for. |

## Item reasons

| reason | State | Meaning |
|---|---|---|
| `RULE_IMPORT_GROUP_UNTRANSLATABLE` | `SAVED_FAILED_TRANSLATION` | Saved as a disabled starter; `ERROR` facts say why. |
| `RULE_IMPORT_ALREADY_EXISTS` | `SKIPPED_EXISTS` | A rule of either name exists (a re-run, the same name twice in one source, or two imports at once); nothing written. |
| `LIFECYCLE_PERSON_TARGETING_PERMISSION_REQUIRED` | `FAILED` | The group reads facts about people; the account needs `MANAGE_PERMISSIONS`. |
| `LIFECYCLE_QUALITY_PROFILE_NOT_FOUND` | `FAILED` | The quality profile the rule sets is gone from Scryer. |
| `RULE_IMPORT_ITEM_FAILED` | `FAILED` | Saving failed for another reason; item `message` says why. |
| `RULE_IMPORT_INTERRUPTED` | `FAILED` | Not saved before Scryer stopped. |

Any other `LIFECYCLE_*` reason on a `FAILED` item is the lifecycle rule save's
own refusal; `scryer-connect/references/errors.md` lists them.

## Diagnostic facts

Each has `details.severity` (`ERROR` makes the group untranslated, `WARNING`
does not) and `details.ruleIndex` (the Maintainerr rule, from 0, or null for
the whole group).

| code | Severity | Meaning |
|---|---|---|
| `maintainerr.group.invalid` | ERROR | The group's rules, as Maintainerr combines them, can never match. |
| `maintainerr.group.rules_disabled` | ERROR | The group does not use rules. |
| `maintainerr.group.rules_empty` | ERROR | The group has no rules. |
| `maintainerr.group.rules_too_many` | ERROR | More rules than one lifecycle rule holds. |
| `maintainerr.group.media_type_unsupported` | ERROR | Media type missing or not one Scryer acts on. |
| `maintainerr.rule.invalid` | ERROR | A rule is malformed. |
| `maintainerr.rule.second_value_missing` | ERROR | A rule compares with another property that is missing. |
| `maintainerr.rule.username_missing` | ERROR | A per-user rule names no user. |
| `maintainerr.application.unknown` | ERROR | A rule reads an application Maintainerr does not list. |
| `maintainerr.property.unknown` | ERROR | A rule reads a property Maintainerr does not list. |
| `maintainerr.property.unmapped` | ERROR | No faithful Scryer fact for the property. |
| `maintainerr.property.collection_unsupported` | ERROR | Media-server collections, playlists or labels. |
| `maintainerr.property.level_unsupported` | ERROR | Scryer has the fact only for another media type. |
| `maintainerr.comparator.unsupported` | ERROR | The comparison cannot be expressed exactly. |
| `maintainerr.comparator.date_equality_unsupported` | ERROR | Date equality depends on Maintainerr's time zone. |
| `maintainerr.comparator.partial_list_widens` | ERROR | Scryer holds only part of the list, and this comparison (not contains, a count at most, equals) could match more titles. |
| `maintainerr.comparator.absence_widens` | ERROR | The comparison matches a value being absent or unset, which Scryer can be where Maintainerr is not. |
| `maintainerr.comparator.date_drift_widens` | ERROR | Scryer's date can be later (or either way) than Maintainerr's, in the direction this comparison would match more titles. |
| `maintainerr.username.unknown_widens` | ERROR | A per-user play count names no Scryer user, and zero plays satisfy the comparison. |
| `maintainerr.custom_value.invalid` | ERROR | A value cannot be read as Maintainerr reads it. |
| `maintainerr.action.missing` | ERROR | The group has no action. |
| `maintainerr.action.unsupported` | ERROR | No Scryer action for this media type. |
| `maintainerr.quality_profile.unresolved` | ERROR | The group's profile is not one Sonarr or Radarr lists. |
| `maintainerr.quality_profile.not_in_scryer` | ERROR | No Scryer profile has that name. |
| `maintainerr.source.too_large` | ERROR | The translated rule is over the size limit. |
| `maintainerr.source.invalid` | ERROR | The translated rule failed validation. |
| `maintainerr.username.unknown` | WARNING | A rule names a user no Scryer user is called; it translates and matches nothing for that user until one is. |
| `maintainerr.fact.approximate` | WARNING | The Scryer fact differs from Maintainerr's as the message states; the comparison can only match fewer titles. |
| `maintainerr.scope.all_libraries` | WARNING | The group was tied to one media-server library; the rule applies to every Scryer library. |
| `maintainerr.action.approximate` | WARNING | The Scryer action differs from Maintainerr's as the message states. |
