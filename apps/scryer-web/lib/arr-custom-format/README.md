# Arr custom-format import

The settings action lazily opens the import dialog. Inspection loads on Review;
the compiler and its dependencies load in a dedicated worker only on Translate.
Closing the dialog terminates that worker. Import creates an unsaved editor
draft and uses the existing dirty-draft confirmation and Rego validation flow.

`inspection.ts` normalizes single/batch exports, Arr field arrays and TRaSH
field objects, score choices, and structural diagnostics. The compiler groups
conditions by implementation: each group requires every required condition, or
at least one optional condition when none are required; all groups must match.
Unsupported conditions disable their entire format, including optional or
negated conditions. The output retains a false-guarded score entry with a reason.

`specifications.ts` maps verified Sonarr/Radarr enums to the existing rule input.
Unrepresentable values are disabled. Year conditions are disabled because
Radarr's movie-metadata fallback is unavailable. Original language uses actual
original-language metadata rather than the inferred audio-language field.

The compiler constructs `rego-deparser` AST nodes for all executable output.
`oniguruma-parser` supplies a structural regex AST. Supported lookarounds become
relations over original string offsets, preserving local assertions, alternatives,
anchors, empty matches, and overlapping candidates. Fixed-width fragments avoid
enumerating every possible end offset. Variable-width fragments can still require
quadratic substring matching; large imports may need to be split into smaller
rules. Capture-dependent regexes, unsupported flags, and quantified assertions
are disabled rather than approximated.

The pinned corpus benchmark requires at least 97% of whole formats to translate;
disabled entries stay in its denominator. The test-only AST interpreter checks
emitted predicate behavior against independent regex examples. It is not an OPA
or Rust validator. Runtime validation remains the editor's responsibility.

Compatibility does not imply identical media parsing: Scryer release fields come
from its own parser. In particular there is no separate Arr filename fallback,
and regex Unicode/culture behavior follows the target engine. See
[`__fixtures__/README.md`](__fixtures__/README.md) for corpus provenance.
