---
name: scryer-connect
description: Connects an agent to an operator's own Scryer instance over its GraphQL API with a personal API key, verifies the connection with one cheap query, and sets the rules every other Scryer skill relies on (the gql.sh runner, error codes, token discipline, confirmation before changes). Use it first whenever an operator asks to inspect, configure, migrate to, or automate their Scryer server, or when another Scryer skill says "Complete scryer-connect first".
---

# Connect to Scryer

One GraphQL endpoint: `POST $SCRYER_URL/graphql`, header
`Authorization: Bearer <api key>`. Every Scryer skill calls it via
`scripts/gql.sh`. Do not read the schema or Scryer's source; each skill ships
its exact operations under `operations/`.

## 1. Get an API key (operator does this)

Operator creates the key in the Scryer web UI:

1. Sign in as an account with the permissions the task needs (below).
2. **Settings → Profile → API keys** → **Create API key**.
3. Label (e.g. `agent-migration`) and expiry. Default 90 days; 30, 365 days
   and never also offered.
4. Key shown once. Starts with `ska`.

An API key cannot create, list or revoke keys; that needs interactive
sign-in, so key management stays in the web UI. If the account has MFA,
Scryer asks for a code before creating the key.

## 2. Permissions that matter

Key acts for the account that created it, with that account's app
permissions. Full table: `references/permissions.md`. Briefly:

- `MANAGE_CATALOG_SETTINGS`: libraries, quality profiles, rules, delay
  profiles, routing, arr import preview and finalize steps.
- `MANAGE_SYSTEM_SETTINGS`: download clients, indexers, notifications,
  creating indexers and clients during arr import.
- `MANAGE_USERS`, `MANAGE_PERMISSIONS`, `MANAGE_LISTS`: only those areas.

First admin account has all of them.

**MFA step-up blocks API keys.** When form login is on and **Settings →
Security → Require MFA step-up for configuration changes** is enabled, every
configuration mutation needs a recent MFA check an API key cannot carry.
Those calls fail with `MFA_STEP_UP_REQUIRED` or `TOTP_ENROLLMENT_REQUIRED`.
Read-only queries still work. Tell the operator the setting blocks automated
configuration. Only they decide whether to turn it off for the session.
Never change it yourself.

## 3. Set the environment

Operator sets these in the agent's shell. Do not ask them to paste the key
into the conversation.

```sh
export SCRYER_URL="http://scryer.example.lan:8080"   # no trailing slash
export SCRYER_API_KEY="..."                          # the ska... key
```

Base path (e.g. `/scryer`) goes in `SCRYER_URL`. `gql.sh` appends `/graphql`.

Needs: POSIX `sh`, `curl`, `jq` 1.6+.

## 4. Verify with one cheap query

```sh
S=path/to/skills/scryer-connect
"$S/scripts/gql.sh" "$S/operations/Me.graphql" > me.json && \
  jq -c '.data.me | {username, appPermissions}' me.json
```

Success prints username and permission list. Compare permissions with the
task's needs before going on. Do not use `systemHealth` as a connection check
(heavy); use `Me`.

## 5. The runner: scripts/gql.sh

```
gql.sh <operation-file.graphql> [variables-json-file] > out.json
```

- Writes full response body to stdout; always redirect to a file.
- Exit 0 success. Exit 1 response carried GraphQL errors. Exit 2 transport
  failure, non-JSON, or HTTP-level refusal. Exit 64 usage or environment
  problem.
- On errors prints one line per error to stderr:
  `<extensions.code> [<extensions.reason>] <message>`. Rule diagnostics follow
  as `line L col C <code>: <message>`.
- Key goes to curl on stdin; never in argv, output or logs.
- Write variables with `jq -n ... > vars.json`. Do not hand-edit JSON that
  contains secrets.

### MCP instead of the runner

Same operations over MCP at `POST $SCRYER_URL/mcp` (streamable HTTP,
stateless), same `Authorization: Bearer <api key>` header. Use when the agent
has an MCP client and no shell. Operator adds the server to the client's
configuration, so the key stays out of the conversation.
On by default; **Settings → Security & Access → Allow the MCP server** turns
it off, and every request then gets HTTP 403 with `reason`
`HTTP_MCP_DISABLED`. `SCRYER_MCP_ENABLED=true|false` overrides that setting
while set (the switch then shows the forced value, locked). Only the operator
turns it back on; use `gql.sh` meanwhile.

- `whoami` replaces the section 4 check.
- `list_operations` (optionally one `skill`) names operations;
  `describe_operation` gives one operation's variables and types.
- `run_query` / `run_mutation` take `name`, `variables`, and `skill` when two
  skills ship different text under one name. Only operations under a skill's
  `operations/` directory run.
- `read_diagnostics_section` and `search_service_logs` read diagnostics
  without downloading the bundle.

A step `gql.sh operations/X.graphql vars.json` = `run_query` or
`run_mutation` with `name: "X"` and the same variables. Result over 32 KiB is
refused with `reason` `HTTP_MCP_RESULT_TOO_LARGE`: narrow with paging or
filter variables. Every section 8 rule applies unchanged.

## 6. Reading errors

Branch on `extensions.code` and `extensions.reason`, never on message text
(for people, can change). Codes and actions: `references/errors.md`. Common:

- `AUTHENTICATION_REQUIRED` (HTTP 401): key missing, wrong, expired or
  revoked → ask operator for a new key.
- `UNAUTHORIZED`: account lacks a permission → name the permission, stop.
- `MFA_STEP_UP_REQUIRED`, `TOTP_ENROLLMENT_REQUIRED`: config step-up on → see
  section 2.
- `VALIDATION_ERROR` + `reason`: refused input → fix input for that reason.
- `NOT_FOUND` + `reason`: id does not exist or not visible → re-read ids.
- `TEMPORARY_UNAVAILABLE`: `retryAfterSeconds` may be set → wait, retry once.
- `INTERNAL_ERROR` + `errorId`: server fault → report the `errorId`; do not
  retry in a loop.

## 7. Authless and local-network modes

Scryer can run without login, or skip login for local IPs. Both trust only
the bundled web client (signed header and cookie). A script cannot use them:
without a key, requests get HTTP 401 or 403. Use an API key in every mode.

## 8. Rules every Scryer skill follows

- **Token discipline.** Bulk data to files. Read counts and summaries with
  `jq`, never whole responses. See `references/token-discipline.md`.
- **Secrets.** Never print, echo, log or paste an API key, password or token,
  including arr keys and download-client passwords. Keep them in environment
  variables or mode-600 files that `jq` reads.
- **Confirm before changes.** Before any mutation that changes the server,
  show the operator a short summary of what will change (counts, names) and
  wait for a clear yes. Read-only queries, validation and previews change
  nothing on the server; run them directly.
- **No deletion.** These skills never delete operator files, media or Scryer
  data. If cleanup seems needed, describe it and leave it to the operator.
- **One metadata gateway.** Scryer uses a single central metadata service.
  Nothing to configure or substitute.
- **Use only the hosts the operator names.** Do not try other services
  because a key or URL turned up somewhere.
