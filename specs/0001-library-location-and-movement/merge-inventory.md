# Merge inventory (superseded)

This appendix used to classify every title-referencing table into a per-table
merge disposition. It no longer does. The merge rule is short enough to state in
full here, and the engine implements exactly this:

**The destination title wins everything except two things.**

1. **Media file records.** The merging title's `media_files` rows are repointed
   at the destination title. The rows that belong to a file travel with it,
   because `media_files.id` never changes across a merge:
   - `file_episode_map` — the episode side is remapped through the identity map
     and the role is resolved per slot (FR-068 to FR-070);
   - `file_series_movie_link_map` — the link id is remapped, the file id is not;
   - every other media-file-keyed dependent (subtitle blocklist entries, media
     info, hashes) follows the file with no rewrite at all.

   A file that hangs off the title rather than off an episode — a movie's file —
   resolves its `media_files.role` against the destination's title slot under the
   same rule.

2. **History.** `history_events.title_id` is repointed, and `domain_events` has
   its `title_id`, its title `stream_id`, and the `$.data.episode_ids[]` /
   `$.data.collection_id` inside the payloads that carry them remapped.

**Everything else recorded against the merging title retires with it**, through
the ordinary title-delete path (`purge_title_dependent_records` for the rows no
foreign key reaches, then the cascade from `DELETE FROM titles`). That covers
tags, wanted items, download submissions and their import artifacts, the
blocklist, requests, subtitle downloads, workflow operations, indexer learning
and coverage, discovery provenance, lifecycle candidates and action runs,
maintenance-rule exclusions, watch signals, unmatched scan items, external ids,
images, credits, and metadata tags. There is no per-table disposition table and
no foreign-key gate: a retired title is a retired title, and the delete path
already owns what one leaves behind.

**What blocks.** Only a slot the merge is carrying something onto has to map: an
episode, collection, or series-movie link named by a source media file record or
by a history row. An unmapped or ambiguous slot of that kind refuses the merge; a
source slot carrying nothing the merge moves is simply not mapped. A merge is
also refused while the merging title has active acquisition work (a queued or
in-flight download, an unconsumed manual-import selection) or while another
resumable location operation still holds it.

See FR-063 to FR-071 in `spec.md`.
