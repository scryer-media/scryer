# Download clients

Needs **Manage system settings** for client config and tests, **Manage
catalog settings** for routing. UI: **Integrations > Download clients**
(`/integrations/download-clients`); per-scope routing under each media
area's settings, **Routing**.

## Read

```sh
sh "$G" "$S"/operations/ListDownloadClients.graphql > clients.json
jq -r '.data.downloadClientConfigs[] | "\(.id)\t\(.name)\t\(.clientType)\tenabled=\(.isEnabled)\tstatus=\(.status)"' clients.json
```

List order = client priority order.

## Fields

- `name` (string): required on create.
- `clientType` (string): `providerType` from `DownloadClientProviderTypes`;
  required on create.
- `isEnabled` (bool).
- `proxyConfigId` (id): must be an enabled proxy.
- `config` ([ProviderConfigValueInput]): connection keys below plus the
  provider's own fields.

Connection keys Scryer reads for every client, besides the provider's
declared fields:

| key | input | meaning |
|---|---|---|
| `host` | stringValue | host name or address as seen from the Scryer host or container |
| `port` | stringValue | port number as text |
| `use_ssl` | boolValue | https |
| `url_base` | stringValue | path prefix behind a reverse proxy |
| `remote_path_mappings` | stringValue | see below |
| `client_type` | stringValue | same as `clientType` |
| `username`, `password`, `api_key` | secretValue | only when the provider does not declare its own field for it |

Empty values are left out. A provider that declares its own credential
fields owns them; send those keys instead.

### Remote path mappings

For when the client reports paths that differ from what Scryer sees (e.g.
separate containers). One per line, `REMOTE => LOCAL`:

```
/downloads => /data/downloads
/incomplete => /data/incomplete
```

Local side must be absolute. Refusals start with
`DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_` (`refusal-codes.md`).

### Update replaces config

`config` on update replaces every non-secret key. Start from
`config-to-input.sh clients.json "$ID"`, change only requested keys, send
the full array. Omitted secret keys keep their stored value.

## Add a client

1. Discover the provider's fields (`plugin-config-fields.md`).
2. Build `config.json` with connection keys and provider fields; add secrets
   with `add-secret.sh`.
3. Create does **not** test the connection. Offer a test first (contacts the
   client; ask):
   `jq -n --arg t "$TYPE" --slurpfile c config.json '{input: {clientType: $t, config: $c[0]}}' > test.json`
   then `TestDownloadClientConnection.graphql`.
4. `jq -n --arg n "Example Client" --arg t "$TYPE" --slurpfile c config.json '{input: {name: $n, clientType: $t, config: $c[0]}}' > create.json`.
   Show every non-secret value; wait for a yes.
5. `CreateDownloadClient.graphql`. Create also adds a routing entry; review
   it with `DownloadClientRouting`.

## Edit a client

1. `jq --arg id "$ID" '.data.downloadClientConfigs[] | select(.id == $id)' clients.json > before.json`
2. Write `update.json` (`{"input": {"id": "...", ...}}`).
3. `show-change.sh before.json update.json`, yes, then
   `UpdateDownloadClient.graphql`.

A test with `id` set fills stored secrets into the test; no re-entry needed.

## Order

`ReorderDownloadClients.graphql` with `{"input": {"ids": [...]}}`, every
client id in new order. Show old and new order; ask.

## Routing (per scope)

`downloadClientRouting(scope)` returns, per client:

- `enabled`: use this client for this scope.
- `category`: client's category or label; `DownloadClientCategories.graphql`
  lists what the client reports (contacts the client).
- `recentQueuePriority`, `olderQueuePriority`: one of `force`, `very high`,
  `high`, `normal`, `low`, `very low`.
- `removeCompleted`: Scryer removes the item from the client after import.
- `removeFailed`: Scryer removes failed items from the client.
- `seedingProfileId`: from `SeedingProfiles.graphql` (torrent clients).

**`updateDownloadClientRouting` replaces the whole scope; list order is
priority (first highest).** Send every entry to keep, in order.

Turning `removeCompleted` or `removeFailed` on needs the operator's explicit,
separate approval: say what it removes and from where.

Steps: read with `DownloadClientRouting.graphql` (`{"scope": "SERIES"}`),
save `.data.downloadClientRouting` as `before.json`, edit into
`{"input": {"scope": "SERIES", "entries": [...]}}`, run `show-change.sh`,
get a yes, then `UpdateDownloadClientRouting.graphql`.
