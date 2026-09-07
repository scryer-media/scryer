# TRaSH → Rego snapshot

`arr_custom_formats_rego.json` is a one-time snapshot of the TypeScript
translator's output for all 478 formats in
[`trash-corpus.json`](../../../../apps/scryer-web/lib/arr-custom-format/__fixtures__/trash-corpus.json).
It was generated on 2026-09-07 using the translator in this commit, with a fixed
score of 100 for every format. Generated Rego is not hand-edited.

The source is
[TRaSH-Guides/Guides at 56cb176ef4b59d734d3643287e66aafd7c809bd4](https://github.com/TRaSH-Guides/Guides/tree/56cb176ef4b59d734d3643287e66aafd7c809bd4/docs/json).
The upstream [MIT license](../../../../apps/scryer-web/lib/arr-custom-format/__fixtures__/LICENSE.trash-guides)
is retained with the original JSON corpus.

Generation called `inspectCustomFormats(JSON.stringify(entry.format), entry.source)`
for each source entry, then `translateCustomFormats(inspection.formats,
{ [inspection.formats[0].id]: 100 })`. Each snapshot retains the source path,
application, translation status, diagnostics, and unmodified `regoSource`.

Run from the repository root:

```sh
cargo nextest run --locked -p scryer-application --test arr_custom_format_corpus --success-output immediate
```

The integration test uses the editor's package rewrite and Regorus validator,
which parses, compiles, and dry-runs the rule and its scoring wrapper with
Scryer's builtins and synthetic input. It lives in the application crate so
Regorus receives the same Unicode regex features enabled by the application's
existing dependencies. No dependencies or features were added for this test.

Every emitted snippet must validate, including disabled stubs. Only complete
translations count toward the 97% threshold; disabled formats remain in the
478-format denominator. This does not establish equivalent Arr release parsing
or matching behavior on every possible release.

Validated snapshot: Sonarr 230/236, Radarr 236/242, combined 466/478 (97.49%).
All 478 emitted snippets validate; the 12 disabled stubs are excluded from the
success count. The debug-build dry-run can take several minutes.

The Rust test needs no Node, npm, network access, or live translation step.
It protects this checked-in snapshot; future translator changes require explicit
snapshot regeneration to extend that protection to their emitted output.
