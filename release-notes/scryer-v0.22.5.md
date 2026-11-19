# Scryer 0.22.5 release notes

These changes are relative to v0.22.4.

## Highlights

- A download whose import was retried no longer recycles and rewrites the library's own copy of the same file about once a minute, filling the recycle bin.
- The Balanced, Compatible and Efficient scoring personas penalize oversized releases again, so a normal-sized release is preferred over a much larger one of the same quality.

## Upgrade notes

- Installs that use the bundled TRaSH Guides size rule unchanged receive the updated rule on first start. A copy of the rule you have edited is left as it is.

## Imports

- An episode, movie or series-movie import compares the download with the file it would replace, first by size and then by sampled content. When the two are identical, the import is recorded as already done: nothing is recycled and nothing is written.
- A replacement of a different size is imported as before and reads nothing extra.

## Quality and scoring

- Once the runtime is known, the bundled size rule scores a release above the persona's size band negatively. Balanced and Compatible start at 1.25 times the expected size. Efficient and the compact override start at 1.1 times and fall more steeply. The penalty stops at -1200.
- Audiophile, a release whose runtime is unknown and a pack judged on its member size are not penalized.
- A file already in the library is scored by the same rule, so an oversized file can be upgraded to a normal-sized release.
- The Balanced, Compatible and Efficient persona cards describe the oversize penalty instead of a preference for larger or standard sizes.
