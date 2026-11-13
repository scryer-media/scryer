# Feature form fields

The repository's feature form (`.github/ISSUE_TEMPLATE/feature_request.yml`)
has a title prefix of `[Feature]: ` and the label `enhancement`. Draft
exactly these four fields, in this order, with these headings.

| # | heading | required | what goes in it |
|---|---|---|---|
| 1 | Is your feature request related to a problem? | yes | the operator's problem in their terms: what they are trying to do, what gets in the way, how often. No proposed design. |
| 2 | Describe the solution you'd like | yes | the outcome they want, as behaviour they would observe ("I can see...", "Scryer stops..."), not screens, settings or code |
| 3 | Describe alternatives you've considered | no | what they tried in Scryer today and why it falls short, including any existing setting found in step 1 of the skill |
| 4 | Additional context | no | scale (number of titles, indexers, users) as counts, the Scryer version, and the comparable behaviour in another tool if the operator mentions one |

## Keep it about the problem

- Lead with the use case: who is affected, when, and what it costs them.
- Write the solution as an outcome. If the operator describes a specific
  UI or setting, keep it as one example sentence ("for example, a per-library
  toggle") and keep the outcome as the main ask.
- One request per draft. Split unrelated asks into separate drafts.
- No release names, host names, paths, keys or private indexer names.

## Title

`[Feature]: ` followed by the outcome in a few words, for example
`[Feature]: Hold upgrades for titles still being watched`.

## Draft file layout

Write the draft to `./scryer-feature-request.md`:

```
Title: [Feature]: <outcome>

### Is your feature request related to a problem?
...

### Describe the solution you'd like
...

### Describe alternatives you've considered
...

### Additional context
...
```
