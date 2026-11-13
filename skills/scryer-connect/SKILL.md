---
name: scryer-connect
description: Connects an agent to an operator's own Scryer instance over its GraphQL API with a personal API key, verifies the connection with one cheap query, and sets the rules every other Scryer skill relies on (the gql.sh runner, error codes, token discipline, confirmation before changes). Use it first whenever an operator asks to inspect, configure, migrate to, or automate their Scryer server, or when another Scryer skill says "Complete scryer-connect first".
---

# Connect to Scryer

Scryer exposes one GraphQL endpoint: `POST $SCRYER_URL/graphql`, authenticated
with `Authorization: Bearer <api key>`. Every Scryer skill talks to it through
`scripts/gql.sh`. Do not read the schema or Scryer's source. Each skill
ships the exact operations it needs under `operations/`.

## 1. Get an API key (operator does this)

Ask the operator to create a key in the Scryer web UI:

1. Sign in as an account with the permissions the task needs (below).
2. Open **Settings → Profile → API keys** and choose **Create API key**.
3. Give it a label (for example `agent-migration`) and an expiry. The default
   expiry is 90 days. 30, 365 days and never are also offered.
4. Scryer shows the key once. It starts with `ska`.

An API key cannot create, list or revoke keys. Those need an interactive
sign-in, so key management always stays in the web UI. If the account has
MFA, Scryer asks for a code before it creates the key.

## 2. Permissions that matter

A key acts for the account that created it and carries that account's app
permissions. Read `references/permissions.md` for the full table. Briefly:

- `MANAGE_CATALOG_SETTINGS`: libraries, quality profiles, rules, delay
  profiles, routing, the arr import's preview and finalize steps.
- `MANAGE_SYSTEM_SETTINGS`: download clients, indexers, notifications,
  creating indexers and clients during the arr import.
- `MANAGE_USERS`, `MANAGE_PERMISSIONS`, `MANAGE_LISTS`: only for those areas.

The first admin account has all of them.

**MFA step-up blocks API keys.** When form login is on and **Settings → Security → Require MFA
step-up for configuration changes** is enabled, every configuration
mutation needs a recent MFA check that an API key cannot carry. Those calls fail
with `MFA_STEP_UP_REQUIRED` or `TOTP_ENROLLMENT_REQUIRED`. Read-only queries
still work. Tell the operator that the setting blocks automated configuration.
Only they can decide to turn it off for the session. Never change it yourself.

## 3. Set the environment

The operator sets these in the shell the agent uses. Do not ask them to paste
the key into the conversation.

```sh
export SCRYER_URL="http://scryer.example.lan:8080"   # no trailing slash
export SCRYER_API_KEY="..."                          # the ska... key
```

If Scryer is served under a base path (for example `/scryer`), include it in
`SCRYER_URL`. `gql.sh` appends `/graphql`.

Needs: POSIX `sh`, `curl`, and `jq` 1.6 or later.

## 4. Verify with one cheap query

```sh
S=path/to/skills/scryer-connect
"$S/scripts/gql.sh" "$S/operations/Me.graphql" > me.json && \
  jq -c '.data.me | {username, appPermissions}' me.json
```

Success prints the username and permission list. Compare the permissions with
the task's needs before going further. `systemHealth` is not a connection check
because it is heavy. Use `Me` instead.

## 5. The runner: scripts/gql.sh

```
gql.sh <operation-file.graphql> [variables-json-file] > out.json
```

- Writes the full response body to stdout, so always redirect it to a file.
- Exit 0 means success. Exit 1 means the response carried GraphQL errors.
  Exit 2 is a transport failure or a non-JSON or HTTP-level refusal. Exit 64 is
  a usage or environment problem.
- On errors it prints one line per error to stderr:
  `<extensions.code> [<extensions.reason>] <message>`. Rule diagnostics follow
  as `line L col C <code>: <message>`.
- The key goes to curl on stdin. It never appears in argv, output or logs.
- Write variables with `jq -n ... > vars.json`. Do not hand-edit JSON that
  contains secrets.

## 6. Reading errors

Branch on `extensions.code` and `extensions.reason`, never on message text.
The message is for people and can change. `references/errors.md` lists the
codes and what to do for each. The common ones:

| code | meaning | action |
|---|---|---|
| `AUTHENTICATION_REQUIRED` (HTTP 401) | key missing, wrong, expired or revoked | ask the operator for a new key |
| `UNAUTHORIZED` | the account lacks a permission | name the permission, stop |
| `MFA_STEP_UP_REQUIRED`, `TOTP_ENROLLMENT_REQUIRED` | config step-up is on | see section 2 |
| `VALIDATION_ERROR` + `reason` | refused input | fix the input for that reason |
| `NOT_FOUND` + `reason` | id does not exist or is not visible | re-read the ids |
| `TEMPORARY_UNAVAILABLE` | retry later; `retryAfterSeconds` may be set | wait, then retry once |
| `INTERNAL_ERROR` + `errorId` | server fault | report the `errorId`; do not retry in a loop |

## 7. Authless and local-network modes

Scryer can run without login, or can skip login for local IPs. Both modes trust
only the bundled web client, which proves itself with a signed header and
cookie. A script cannot use them: without a key, requests are refused with
HTTP 401 or 403. Use an API key in every mode.

## 8. Rules every Scryer skill follows

- **Token discipline.** Bulk data goes to files. Read counts and summaries
  with `jq`, never whole responses. See `references/token-discipline.md`.
- **Secrets.** Never print, echo, log or paste an API key, password or token.
  This covers arr keys and download-client passwords. Keep them in
  environment variables or in mode-600 files that `jq` reads.
- **Confirm before changes.** Before any mutation that changes the server,
  show the operator a short summary of what will change (counts, names) and
  wait for a clear yes. Read-only queries, validation and previews change
  nothing on the server, so they run directly.
- **No deletion.** These skills never delete operator files, media or Scryer
  data. If a cleanup seems needed, describe it and leave it to the operator.
- **One metadata gateway.** Scryer uses a single central metadata service.
  There is nothing to configure or substitute.
- **Use only the hosts the operator names.** Do not try other services
  because a key or URL turned up somewhere.
