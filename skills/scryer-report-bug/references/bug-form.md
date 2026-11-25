# Bug form fields

Bug form (`.github/ISSUE_TEMPLATE/bug_report.yml`): title prefix `[Bug]: `,
label `bug`. Draft exactly these fields, in this order, with these headings.
No others.

1. Describe the bug (required): operator's words, tightened; one or two
   paragraphs.
2. To Reproduce (required): numbered steps; operator confirms them.
3. Expected behavior (required): operator.
4. Screenshots: operator attaches on GitHub; draft says "(attach on GitHub if
   useful)" or is left blank.
5. Installation type: one of `Container`, `Homebrew`, `Binary`; from
   `environment.json`, rule below.
6. Scryer Version: `environment.json` → `.build.version`.
7. Operating System: `environment.json` → `.os.name` and `.os.version` (fall
   back to `.os.family`).
8. Browser: ask only for UI bugs.
9. Browser Version: ask only for UI bugs.
10. Additional context: evidence summary, below.

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

Ask the operator only for a value the bundle lacks.

## Title

`[Bug]: ` plus a short statement of the wrong behaviour, e.g.
`[Bug]: Import fails with FILE_NOT_FOUND when the client uses a remote path mapping`.
No release names, host names or paths in the title.

## Additional context

Short and factual:

- symptom class and codes seen, with counts (e.g.
  `importErrorCode FILE_NOT_FOUND x 4`, `release_decisions.by_code
  quality_blocked: 37`)
- relevant settings as booleans or counts, as the bundle shows them
- at most 20 log lines, already filtered, in a fenced block
- manifest omissions, if any, so maintainers know what the bundle lacks
- whether a diagnostics bundle is available on request

Never paste whole JSON sections, the API key, cookies, URLs with
credentials, or anything the operator marked sensitive.

## Draft file layout

Write to `./scryer-bug-report.md` (or a name the operator picks):

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
