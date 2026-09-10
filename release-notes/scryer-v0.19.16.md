# Scryer 0.19.16 release notes

## Highlights

- A series library whose mount disappears no longer loses its tracked files. Before this release an episodic title scan recreated a missing title directory, walked the empty result and retired every media file record for the title, which made every episode wanted again and could start a mass re-download after a transient mount problem. This closes issue #208. The series scan now follows the same rules Sonarr applies: a library root that is missing or completely empty is treated as unreachable and the title's records are left alone.

## Included fixes

- A title scan never creates the title directory. When the directory is absent and the library root is missing or empty, the scan walks nothing, keeps the title's media file records and unmatched-file rows untouched, and logs one warning naming the title, the root and how many tracked files it kept. A root or title directory that cannot be inspected at all (an unreachable mount, a permission error) fails the scan instead of being treated as empty.
- Deleting a title directory underneath a populated library root is still recognised as a real deletion: its tracked files are retired on the next scan and the episodes become wanted again, exactly as before, but the empty directory is no longer recreated.
- A tracked episode file is retired only when the file is gone and the directory it is confirmed against is still present. Deleting a season folder or a single file underneath an intact title directory is cleaned up as before.

## Upgrading

No migration. No configuration changes.
