/**
 * The keys of the cached media-file maps that list a given file. A deletion
 * targets rows, not files: knowing which episode (or series-movie link) owns
 * the file lets a caller mark exactly those rows pending and hand the same set
 * to the job's terminal handler.
 */
export function mediaFileOwnerKeys(
  fileId: string,
  maps: readonly Record<string, readonly { id: string }[]>[],
): Set<string> {
  const owners = new Set<string>();
  for (const map of maps) {
    for (const [key, files] of Object.entries(map)) {
      if (files.some((file) => file.id === fileId)) {
        owners.add(key);
      }
    }
  }
  return owners;
}
