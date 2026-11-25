# Before anything leaves the machine

Go through every item with the operator, in order, before they file the
report or share the bundle. Nothing here sends anything.

## 1. Say what sanitizing does and does not do

Tell the operator plainly:

- Sanitizing is **best-effort**. Scryer replaces secrets, URLs, email
  addresses, host names, IP addresses and values it knows are configured, but
  cannot promise it caught everything.
- **Release names and file names are kept.** Default path policy
  `keep_file_name` removes directories but keeps the last path component.
  Release and file names can reveal what is in the library.
- Titles, indexer names and other names in `summary.json` appear as
  pseudonyms (`indexer-1(newznab)`, `quality-profile-2`), but log lines can
  still mention titles.

## 2. Show the sanitization section

```sh
jq '.sanitization, .omissions' "$B/manifest.json"
```

Explain each field in one line:

- `passes`: what was scrubbed, in order.
- `path_policy`: `keep_file_name` (file names stay) or `redact_all`.
- `known_values`: how many configured values were replaced.
- `short_values_not_replaced`: configured values too short to replace
  safely; may still appear in the text.
- `vocabulary_values_not_replaced`: configured values that are ordinary
  words, left alone for readability; may still appear.
- Omission `known_values_incomplete:<sources>`: Scryer dropped logs and
  metrics because it could not read every value to scrub.

## 3. Scan for the operator's own sensitive values

Ask: "Which words or values should never appear in what you share? For
example your domain, server name, user name, a private indexer name, or a
title." Operator puts them in a file, one per line, without pasting them into
the conversation:

```sh
sh "$S"/scripts/scan-bundle.sh ./sensitive-terms.txt "$B" ./scryer-bug-report.md
```

Keep the terms file outside the bundle directory and do not read it; it
keeps the values out of the conversation. Script prints only term numbers and
per-file match counts, never terms or matching lines. Also counts built-in
patterns (API-key prefix, bearer tokens, email addresses, IPv4 addresses).

- Every count 0: say so.
- Any count above 0: tell the operator which term number matched in which
  file. Offer to remove matching lines **from the draft report** (the agent's
  own file). For bundle members, the operator decides whether to share the
  bundle at all; do not edit or delete bundle files.

## 4. Decide where the report and bundle go

- Issue goes to the repository's bug form; operator files it.
- Do not attach the bundle to the public issue by default. Issue says a
  bundle is available on request. If a maintainer asks for it, the operator
  chooses how to send it.
- Screenshots: operator checks them for names and addresses before
  attaching.

## 5. Confirm nothing is sent by the agent

Say: "I have not sent, posted or uploaded anything." Only send or post if the
operator explicitly asks in this conversation; then show exactly what will be
sent and where, and wait for a yes.
