# Indexers

Needs the **Manage system settings** permission for indexer config and
tests, and the **Manage catalog settings** permission for routing. UI:
**Integrations > Indexers** (`/integrations/indexers`); per-scope routing
is under each media area's settings, **Routing** (for example
`/movies/settings/routing`).

## Read

```sh
sh "$G" "$S"/operations/ListIndexers.graphql > indexers.json
jq -r '.data.indexers[] | "\(.id)\t\(.name)\t\(.providerType)\tenabled=\(.isEnabled)\tauto=\(.enableAutoSearch)\tinteractive=\(.enableInteractiveSearch)\tmanaged=\(.isManaged)"' indexers.json
```

## Fields

| field | type | notes |
|---|---|---|
| `name` | string | required on create |
| `providerType` | string | from `IndexerProviderTypes`; required on create |
| `isEnabled` | bool | create default true |
| `enableInteractiveSearch` | bool | manual searches; create default true |
| `enableAutoSearch` | bool | automatic and RSS searches; create default true |
| `proxyConfigId` | id | from `ProxyConfigs.graphql`; must be enabled |
| `downloadClientId` | id | pin grabs to one client |
| `rateLimitSeconds`, `rateLimitBurst` | int | request pacing |
| `maxQueriesPerMinute` | int ≥ 1 | a cap per minute |
| `config` | [ProviderConfigValueInput] | provider fields; the base URL is the field whose `role` is CONNECTION_URL |
| `seedingProfileId` | id | read here; set with `setIndexerSeedingProfile` (no operation file shipped; use the UI) |

On update, `proxyConfigId`, `downloadClientId` and `maxQueriesPerMinute`
are tri-state: leave out to keep, `null` to clear, a value to replace.

**`config` on update replaces every non-secret key.** Start from the stored
config, change only what the operator asked, and send the whole array:

```sh
sh "$S"/scripts/config-to-input.sh indexers.json "$ID" > config.json
jq '[.[] | if .key == "base_url" then .stringValue = "https://indexer.example/api" else . end]' config.json > config.new.json
```

Secret keys left out keep their stored value. To replace a secret, use
`add-secret.sh`; to clear one, add `{"key": "...", "clearSecret": true}`.

Managed indexers (`isManaged: true`, synced from an indexer manager) cannot
be edited directly; change them in the manager.

## Add an indexer

1. Discover fields (`plugin-config-fields.md`) and ask for the values.
2. Build `config.json`, adding secrets with `add-secret.sh`.
3. Optional test (contacts the indexer; ask first):
   `jq -n --arg t "$TYPE" --slurpfile c config.json '{input: {providerType: $t, config: $c[0]}}' > test.json`
   then `sh "$G" "$S"/operations/TestIndexerConnection.graphql test.json`.
4. Build the create variables:
   `jq -n --arg n "Example Indexer" --arg t "$TYPE" --slurpfile c config.json '{input: {name: $n, providerType: $t, config: $c[0]}}' > create.json`
5. Show the operator the name, provider, flags and every non-secret config
   value; secrets as `<new secret>`. Create also contacts the indexer to
   validate it. Wait for a yes.
6. `sh "$G" "$S"/operations/CreateIndexer.graphql create.json > created.json`.
   Create adds default routing entries; check them with `IndexerRouting`.

## Edit an indexer

1. Extract the current object: `jq --arg id "$ID" '.data.indexers[] | select(.id == $id)' indexers.json > before.json`.
2. Write `update.json` as `{"input": {"id": "...", <changed fields>}}`,
   including the full `config` only if a config value changes.
3. `sh "$S"/scripts/show-change.sh before.json update.json`; show the output and
   wait for a yes.
4. `sh "$G" "$S"/operations/UpdateIndexer.graphql update.json`.

Testing a saved indexer by id (`indexerId` in the test input) also stores
its capabilities and clears its last error. That is a write as well as a
remote call; ask first.

## Routing (per scope)

`indexerRouting(scope)` returns `{indexerId, enabled, categories, priority}`
for MOVIE, SERIES or ANIME.

| field | notes |
|---|---|
| `enabled` | search this indexer for this scope |
| `categories` | indexer category codes as strings (the indexer's `capsCategories[].code`) |
| `priority` | lower wins a tie |

When two indexers return the same release, the source preference decides
first, then the remaining grab quota, then `priority`.

**`updateIndexerRouting` replaces the whole scope.** An entry left out is
removed from the scope; an empty list clears it. Changing routing also
resets what Scryer has learned about searching the affected indexers.

1. `jq -n '{scope: "MOVIE"}' > scope.json; sh "$G" "$S"/operations/IndexerRouting.graphql scope.json > routing.json`
2. `jq '.data.indexerRouting' routing.json > before.json`
3. Edit into `{"input": {"scope": "MOVIE", "entries": [...all entries...]}}`.
4. `show-change.sh before.json update.json`, yes, then
   `UpdateIndexerRouting.graphql`.

## Download-client pin

`SetIndexerDownloadClientMapping.graphql` with
`{"input": {"indexerId": "...", "downloadClientId": "..." | null}}`. A write;
show before and after and ask.

## Proxies

`ProxyConfigs.graphql` lists proxies. Creating or editing proxies is not
covered here; use **Integrations > Proxies**. Managed indexers cannot use a
proxy, and indexers from an indexer manager cannot use challenge solvers.
