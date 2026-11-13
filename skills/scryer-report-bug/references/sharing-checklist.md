# Before anything leaves the machine

Go through every item with the operator, in order, before they file the
report or share the bundle. Nothing here sends anything anywhere.

## 1. Say what sanitizing does and does not do

Tell the operator plainly:

- The bundle is sanitized on a **best-effort** basis. Scryer replaces
  secrets, URLs, email addresses, host names, IP addresses and the values it
  knows are configured, but it cannot promise it caught everything.
- **Release names and file names are kept.** With the default path policy
  `keep_file_name`, directories are removed but the last path component
  stays. Release and file names can reveal what is in the library.
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
  safely; they may still appear in the text.
- `vocabulary_values_not_replaced`: configured values that are ordinary
  words, left alone to keep the text readable; they may still appear.
- An omission of `known_values_incomplete:<sources>` means Scryer dropped
  the logs and metrics because it could not read every value to scrub.

## 3. Scan for the operator's own sensitive values

Ask: "Which words or values should never appear in what you share? For
example your domain, server name, user name, a private indexer name, or a
title." Ask the operator to put them in a file, one per line, without
pasting them into the conversation:

```sh
sh "$S"/scripts/scan-bundle.sh ./sensitive-terms.txt "$B" ./scryer-bug-report.md
```

Keep the terms file outside the bundle directory and do not read it; it
exists so the values stay out of the conversation. The script prints only
term numbers and per-file match counts, never the
terms or the matching lines. It also counts built-in patterns (API-key
prefix, bearer tokens, email addresses, IPv4 addresses).

- Every count is 0: say so.
- Any count above 0: tell the operator which term number matched in which
  file. Offer to remove the matching lines **from the draft report** (the
  draft is the agent's own file). For bundle members, the operator decides
  whether to share the bundle at all; do not edit or delete bundle files.

## 4. Decide where the report and bundle go

- The issue goes to the repository's bug form; the operator files it.
- Do not attach the bundle to the public issue by default. The issue
  should say a bundle is available on request. If a maintainer asks for
  it, the operator chooses how to send it.
- Screenshots: the operator checks them for names and addresses before
  attaching.

## 5. Confirm nothing is sent by the agent

Say: "I have not sent, posted or uploaded anything." Only send or post if
the operator explicitly asks in this conversation, and then show exactly
what will be sent and where, and wait for a yes.
