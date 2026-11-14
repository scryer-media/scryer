// The request lists are paged by the server. A page shorter than the size
// asked for is the last one.
export const MEDIA_REQUEST_PAGE_SIZE = 200;
// The largest page the server serves in one read.
export const MEDIA_REQUEST_MAX_PAGE_SIZE = 1000;

// Reads every page in order. A request seen on two pages is kept once, at its
// first position. A request that changes while the pages are read can shift
// the rows after it by one, so a walk is only as exact as the list was still;
// the caller reloads when the list changes. A failed page fails the load.
export async function loadAllMediaRequestPages<
  T extends { id: string },
  E = unknown,
>(
  loadPage: (
    limit: number,
    offset: number,
  ) => Promise<{ rows: T[]; error?: E | null }>,
  pageSize: number = MEDIA_REQUEST_PAGE_SIZE,
): Promise<{ rows: T[]; error?: E }> {
  const rows: T[] = [];
  const seen = new Set<string>();
  for (let offset = 0; ; offset += pageSize) {
    const page = await loadPage(pageSize, offset);
    if (page.error) return { rows: [], error: page.error };
    for (const row of page.rows) {
      if (seen.has(row.id)) continue;
      seen.add(row.id);
      rows.push(row);
    }
    if (page.rows.length < pageSize) return { rows };
  }
}
