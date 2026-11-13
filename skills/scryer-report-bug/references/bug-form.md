# Bug form fields

The repository's bug form (`.github/ISSUE_TEMPLATE/bug_report.yml`) has a
title prefix of `[Bug]: ` and the label `bug`. Draft exactly these fields,
in this order, with these headings. Do not add others.

| # | heading | required | source |
|---|---|---|---|
| 1 | Describe the bug | yes | operator's words, tightened; one or two paragraphs |
| 2 | To Reproduce | yes | numbered steps; ask the operator to confirm them |
| 3 | Expected behavior | yes | operator |
| 4 | Screenshots | no | operator attaches them on GitHub; draft says "(attach on GitHub if useful)" or is left blank |
| 5 | Installation type | no | one of `Container`, `Homebrew`, `Binary`; from `environment.json`, rule below |
| 6 | Scryer Version | no | `environment.json` → `.build.version` |
| 7 | Operating System | no | `environment.json` → `.os.name` and `.os.version` (fall back to `.os.family`) |
| 8 | Browser | no | ask only for UI bugs |
| 9 | Browser Version | no | ask only for UI bugs |
| 10 | Additional context | no | evidence summary, below |

## Facts from the bundle, not from the operator

```sh
B=<unpacked bundle dir>
jq '{version: .build.version, os: .os | {family, name, version}, kind: .installation.kind, container: .container, datastore: .datastore.engine}' "$B/environment.json"
```

Installation type:

- `Container` when `.installation.kind` is `docker`, or any of
  `.container.docker_env_file`, `.container.podman_env_file`,
  `.container.kubernetes` is true, or `.container.container_variable` or
  `.container.cgroup_hints` is non-empty.
- `Homebrew` when `.installation.kind` is `homebrew`.
- `Binary` otherwise (portable, MSI, macOS app, winget, Windows supervised).

Only ask the operator for a value the bundle does not have.

## Title

`[Bug]: ` followed by a short statement of the wrong behaviour, for example
`[Bug]: Import fails with FILE_NOT_FOUND when the client uses a remote path mapping`.
No release names, host names or paths in the title.

## Additional context

Keep it short and factual:

- the symptom class and the codes seen, with counts (for example
  `importErrorCode FILE_NOT_FOUND x 4`, `release_decisions.by_code
  quality_blocked: 37`)
- relevant settings as booleans or counts, as the bundle shows them
- at most 20 log lines, already filtered, inside a fenced block
- the manifest omissions, if any, so maintainers know what the bundle lacks
- whether a diagnostics bundle is available on request

Do not paste whole JSON sections, the API key, cookies, URLs with
credentials, or anything the operator marked sensitive.

## Draft file layout

Write the draft to `./scryer-bug-report.md` (or a name the operator picks):

```
Title: [Bug]: <summary>

### Describe the bug
...

### To Reproduce
1. ...

### Expected behavior
...

### Screenshots
...

### Installation type
Container

### Scryer Version
...

### Operating System
...

### Browser
...

### Browser Version
...

### Additional context
...
```
