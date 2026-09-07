# Sourced release-name corpus

This offline corpus contains 1,500 anonymized release-shaped fixtures:
500 movies, 500 series and 500 anime releases. It retains varied filename
syntax and independently reviewed expectations, without collection-site
references or original-name provenance.

Anime coverage consists of 100 episodic releases, 143 unseasoned episode packs,
93 ranges within named seasons, 18 single-season packs, 106 multi-season packs,
14 explicit complete-series releases, 12 generic complete packs, two partial
season packs, three season-extras packs, and nine packs with mixed scopes.
Generic `Complete` or `Batch` markers are not evidence of an entire series.
The source selection was deliberately varied rather than randomly sampled.
It is a regression corpus, not a population accuracy estimate.

## Anonymization

Title, alternate-title, cour-title, episode-title, contributor and release-group
words use alphabetic sentinels. Unknown descriptive text uses `Text` sentinels.
Numeric identity words use numeric sentinels and checksum tags use one fixed
hexadecimal sentinel. Scope words that also occur inside titles, punctuation,
Unicode range/language labels and technical metadata remain to exercise parser
boundaries. No original names, source domains, source IDs, URLs, original-name
hashes, original string lengths or original identity offsets are retained.

Case IDs preserve the 1,500-case denominator and link reviewed annotations to
the frozen mismatch baseline. Some cases have identical anonymized inputs;
they remain separate cases with their independent expected facts. The reviewed
anime annotations retain expected facts, evidence, shapes and review notes.
The Rust harness checks those facts and frozen case IDs. Sanitization is
reviewed alongside fixture changes; no name or source-site list is embedded
in the test code.

The earlier assertion review corrected expected values in 340 anime cases,
anonymous inputs in 189, and contexts in 148; these counts overlap. The later
privacy pass removed residual nontechnical words from four inputs without
changing any expected facts, accepted mismatches or frozen baseline mismatches.
Sentinels change vocabulary and script,
so this corpus does not establish correctness of original title matching or
all Unicode title behavior.

## Assertions

Expected values are derived from source annotations and explicit filename
syntax, without calling the release parser. Tests assert only established
facts, including exact technical fields, episode/season identities, pack
flags and group positions. Movie title numbers are distinguished from release
years, air dates from release years, and `PROPER` from `REPACK`.

Unseasoned anime ranges use absolute coordinates; explicitly season-scoped
ranges use season-relative coordinates. A range plus `Batch` does not prove
that every catalog episode is included, so it must retain exact range coverage.
Likewise, `N Seasons` states a count, not the identities of those seasons; the
corpus does not infer a `1..N` list from that count alone.
The `S2 - 01-N` spelling uses the existing `multi_episode` contract, also with
the entire inclusive episode list; it is not required to be a `range_pack`.
Explicit episode bounds take precedence over completion wording in every
representation: a `$any` range, season-pack, or partial-season alternative
must retain the same inclusive episode list. Completion wording cannot replace
those bounds with unbounded season or series coverage. These conventions follow
the range and season-pack contracts tested in `src/tests.rs` and projected in
`src/parse.rs`.

Mixed sources do not get a guessed single source. Ambiguous combinations of
ordinary episodes, specials and partial seasons do not get invented flat
episode lists or whole-season claims; each such omission is documented in
`anime_annotations.json`. A filename's claimed coverage is not proof of its
actual contents; several listings explicitly contain subtitles only.

Context contains only anonymized target identities; it does not provide
expected years, episodes or season mappings to the parser. Every case has at
least two assertion checks. An absent assertion means unknown, not a negative
assertion. A `$any` comparison counts as one compound check in reports.

Tests accumulate all mismatches. A reviewed `accepted_failure` may document a
known limitation only as an exact signature of observed fields, values, and
missing-field flags. It remains an accuracy failure; an extra, changed, or
missing mismatch, or a newly passing accepted case, fails the gate. There are
no parser-derived expected facts, ignored cases, or network calls in the test
path. Only accepted-mismatch signatures record observed parser values.

```sh
SCRYER_PARSER_CORPUS_REPORT_DIR=/tmp/parser-corpus-report \
  cargo nextest run -p scryer-release-parser --test sourced_release_corpus --no-fail-fast
```

## Measurement after the anime assertion review

Measured against `dd37b3c2d` plus this corpus, with no parser implementation
changes during the assertion review. The initial anime result of 277/500 is
withdrawn because its setup included inaccurate and contradictory assertions.
The reviewed corpus has 38 formerly failing cases that pass and three formerly
passing cases that fail. These numbers measure the asserted facts, not every
field the parser can produce.

| Category | Entire cases passing | Assertion checks passing |
| --- | ---: | ---: |
| Movie | 500 / 500 | 2,917 / 2,917 |
| Series | 472 / 500 | 4,354 / 4,382 |
| Anime | 312 / 500 | 3,082 / 3,851 |
| Total | 1,284 / 1,500 | 10,353 / 11,150 |

Series mismatches are 27 missing year fields and one release-group mismatch.
Subsequent fixture calibration removed the explicit year from those 27 title
contexts: the release name and expected year were unchanged. Case `anime-0389`
now asserts its explicit absolute range, 1-200, alongside the named seasons
without requiring unbounded whole-season coverage; its exact deferred
named-season representation is recorded as an accepted failure. Its unresolved
scope diagnostic blocks automatic coverage while that restriction cannot be
attached safely. Case
`anime-0492` likewise retains named seasons and its explicit global E001-E316
range without inferring per-season coordinates or whole-season coverage. Case
`anime-0309` does not treat one S00E01 special as complete season zero. The
table above preserves the historical measurement before those corrections and
parser changes.

Case `anime-0354` originally asserted full seasons despite its explicit
001-068 bounds. The corrected assertion retains seasons 1-3, requires the
inclusive global episode bounds, and rejects whole-season coverage. This
correction is documented beside the fixture and in its reviewed annotation;
the frozen baseline remains unchanged. Accepted-failure exemptions for
`anime-0381`, `anime-0482`, and `anime-0492` were removed after reviewing general
group-punctuation, known-alias, and bounded season-annotation fixes. Their
original expected facts remain unchanged.

Case `anime-0416` intentionally stays unresolved: its season marker is fused
into a title token, followed by several aliases, Part 2 and cour names before
the episode bounds. Splitting the title and carrying season scope across that
text would require a separate title-boundary grammar. Its exact accepted
mismatch is documented beside the fixture; all original episode facts and
technical metadata assertions still run and it remains an accuracy failure.

Historical anime mismatches concentrated in explicit ranges within season packs,
multi-season identities, complete-series markers, and technical metadata
around multiple aliases. Episodic anime passed 96/100; packs passed 216/400.
Failures among ranges within named seasons predominantly lost explicit episode
coverage or claimed a whole season. Both explicitly partial season packs passed.
The 500 anime cases produce 459 distinct anonymous
inputs; repeats after anonymization remain represented by separate case IDs. Failures identify behavior to investigate and do not
establish that every mismatch is a regression introduced in one release.

Validation results are reported by the Rust corpus harness, including accepted,
unexpected, stale-signature, and unexpected-pass counts.
`baseline.json` freezes the original case IDs and mismatch signatures used for
implementation comparison; it is not an oracle and does not replace expected
facts.

After the bounded pack fixes, all 1,500 fixtures still execute:

| Category | Passing | Accepted failing | Unexpected failing | Total |
| --- | ---: | ---: | ---: | ---: |
| Movie | 500 | 0 | 0 | 500 |
| Series | 500 | 0 | 0 | 500 |
| Anime | 488 | 12 | 0 | 500 |
| Total | 1,488 | 12 | 0 | 1,500 |

Accuracy is 99.2% overall and 97.6% for anime. There are no unexpected passes
or stale accepted signatures. The 12 accepted cases remain accuracy failures.
The frozen baseline comparison has 176 formerly failing anime cases now passing
and no newly failing baseline cases. This comparison includes the independently
documented assertion corrections above; it is not a pure parser-only metric.
