# Indexers

Needs **Manage system settings** for indexer config and tests, **Manage
catalog settings** for routing. UI: **Integrations > Indexers**
(`/integrations/indexers`); per-scope routing under each media area's
settings, **Routing** (e.g. `/movies/settings/routing`).

## Read

```sh
sh "$G" "$S"/operations/ListIndexers.graphql > indexers.json
jq -r '.data.indexers[] | "\(.id)\t\(.name)\t\(.providerType)\tenabled=\(.isEnabled)\tauto=\(.enableAutoSearch)\tinteractive=\(.enableInteractiveSearch)\tmanaged=\(.isManaged)"' indexers.json
```

## Fields

- `name` (string): required on create.
- `providerType` (string): from `IndexerProviderTypes`; required on create.
- `isEnabled` (bool): create default true.
- `enableInteractiveSearch` (bool): manual searches; create default true.
- `enableAutoSearch` (bool): automatic and RSS searches; create default true.
- `proxyConfigId` (id): from `ProxyConfigs.graphql`; must be enabled.
- `downloadClientId` (id): pin grabs to one client.
- `rateLimitSeconds`, `rateLimitBurst` (int): request pacing.
- `maxQueriesPerMinute` (int ≥ 1): cap per minute.
- `maxRequestsPerMonth`, `maxGrabsPerMonth` (int ≥ 1): caps on what this
  instance sends per UTC calendar month; unset = no limit.
- `config` ([ProviderConfigValueInput]): provider fields; base URL is the
  field whose `role` is CONNECTION_URL.
- `seedingProfileId` (id): read here; set with `setIndexerSeedingProfile`
  (no operation file shipped; use the UI).

On update, `proxyConfigId`, `downloadClientId`, `maxQueriesPerMinute`,
`maxRequestsPerMonth` and `maxGrabsPerMonth` are tri-state: omit to keep,
`null` to clear, value to replace.

**`config` on update replaces every non-secret key.** Start from stored
config, change only what was asked, send the whole array:

```sh
sh "$S"/scripts/config-to-input.sh indexers.json "$ID" > config.json
jq '[.[] | if .key == "base_url" then .stringValue = "https://indexer.example/api" else . end]' config.json > config.new.json
```

Omitted secret keys keep their stored value. Replace a secret with
`add-secret.sh`; clear one with `{"key": "...", "clearSecret": true}`.

Managed indexers (`isManaged: true`, synced from an indexer manager) cannot
be edited directly; change them in the manager.

## Add an indexer

1. Discover fields (`plugin-config-fields.md`); ask for values.
2. Build `config.json`, adding secrets with `add-secret.sh`.
3. Optional test (contacts the indexer; ask first):
   `jq -n --arg t "$TYPE" --slurpfile c config.json '{input: {providerType: $t, config: $c[0]}}' > test.json`
   then `sh "$G" "$S"/operations/TestIndexerConnection.graphql test.json`.
4. Create variables:
   `jq -n --arg n "Example Indexer" --arg t "$TYPE" --slurpfile c config.json '{input: {name: $n, providerType: $t, config: $c[0]}}' > create.json`
5. Show name, provider, flags and every non-secret config value; secrets as
   `<new secret>`. Create also contacts the indexer to validate it. Wait for a
   yes.
6. `sh "$G" "$S"/operations/CreateIndexer.graphql create.json > created.json`.
   Create adds default routing entries; check with `IndexerRouting`.

## Edit an indexer

1. `jq --arg id "$ID" '.data.indexers[] | select(.id == $id)' indexers.json > before.json`.
2. Write `update.json` as `{"input": {"id": "...", <changed fields>}}`; full
   `config` only if a config value changes.
3. `sh "$S"/scripts/show-change.sh before.json update.json`; show output, wait
   for a yes.
4. `sh "$G" "$S"/operations/UpdateIndexer.graphql update.json`.

Testing a saved indexer by id (`indexerId` in the test input) also stores its
capabilities and clears its last error: a write plus a remote call. Ask
first.

## Routing (per scope)

`indexerRouting(scope)` returns `{indexerId, enabled, categories, priority}`
for MOVIE, SERIES or ANIME.

- `enabled`: search this indexer for this scope.
- `categories`: indexer category codes as strings (the indexer's
  `capsCategories[].code`).
- `priority`: lower wins a tie.

Same release from two indexers: source preference decides first, then
remaining grab quota, then `priority`.

**`updateIndexerRouting` replaces the whole scope.** An omitted entry is
removed from the scope; an empty list clears it. Changing routing also resets
what Scryer has learned about searching the affected indexers.

1. `jq -n '{scope: "MOVIE"}' > scope.json; sh "$G" "$S"/operations/IndexerRouting.graphql scope.json > routing.json`
2. `jq '.data.indexerRouting' routing.json > before.json`
3. Edit into `{"input": {"scope": "MOVIE", "entries": [...all entries...]}}`.
4. `show-change.sh before.json update.json`, yes, then
   `UpdateIndexerRouting.graphql`.

## Download-client pin

`SetIndexerDownloadClientMapping.graphql` with
`{"input": {"indexerId": "...", "downloadClientId": "..." | null}}`. A write;
show before and after, ask.

## Proxies

`ProxyConfigs.graphql` lists proxies. Creating or editing proxies: not
covered; use **Integrations > Proxies**. Managed indexers cannot use a proxy;
indexers from an indexer manager cannot use challenge solvers.
