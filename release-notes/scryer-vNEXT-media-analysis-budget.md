# scryer-vNEXT — media analysis keeps what it read

Fold this section into the next release's notes file when the version is cut.

## Fixed
- **Files no longer report every format as "Unknown."** Media analysis works to a
  per-file read and I/O budget so that scanning a library stays fast. When that
  budget ran out *after* a file's tracks had already been read — while sampling
  frames, accounting an index, or probing streams for extra detail — the
  analysis threw away everything it had found and saved an empty result. The
  file then showed no codec, no resolution, no channel layout and no HDR format,
  and the Details panel reported nothing read at all. Large Matroska files were
  affected most, because they always run the deep probe.

  Analysis now stops at the stage where the budget ran out and keeps the tracks
  it had already read. The file reports its real video codec, audio channels and
  HDR format, and its analysis is marked incomplete with a warning naming the
  stage that was cut short, so a partial report is still visibly partial. This
  applies to Matroska, AVI, MPEG-TS and FLV files alike; the scanning budget
  itself is unchanged.

  Affected files are re-analyzed automatically by the daily background pass, so
  no rescan is required.
