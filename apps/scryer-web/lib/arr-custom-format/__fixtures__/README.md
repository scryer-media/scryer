# Arr compatibility fixtures

`trash-corpus.json` contains all 478 custom-format definitions from
`docs/json/radarr/cf/` and `docs/json/sonarr/cf/` in
[TRaSH-Guides/Guides at 56cb176ef4b59d734d3643287e66aafd7c809bd4](https://github.com/TRaSH-Guides/Guides/tree/56cb176ef4b59d734d3643287e66aafd7c809bd4/docs/json).
Each entry retains its application, upstream path, and original JSON object.
The upstream MIT license is included in `LICENSE.trash-guides`.

These fixtures are test-only and are never imported by the application or worker.
Tests do not fetch upstream data. Changes to the pinned revision or corpus size
must be reviewed explicitly; unsupported formats remain in the denominator.

Run the complete-format compatibility benchmark with:

```sh
node --test lib/arr-custom-format/corpus.test.ts
```

The benchmark measures whether every condition in a format has a supported
translation. Disabled stubs count as failures. A 97% result is a compatibility
measurement, not proof of equivalent release parsing between different media
applications. Separate semantic tests cover condition grouping, negation,
regex assertions, and numeric boundaries. The existing editor validates the
generated Rego when the user validates or saves the draft.
