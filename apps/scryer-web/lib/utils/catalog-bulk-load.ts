/** Fetch a complete listing in bounded waves, publishing rows only on success. */
export async function loadCatalogBulk<T>(options: {
  offset: number;
  pageSize: number;
  isCurrent: () => boolean;
  loadPage: (offset: number) => Promise<{ items: T[]; hasMore: boolean }>;
  onProgress: (loaded: number) => void;
}): Promise<T[] | null> {
  const items: T[] = [];
  let offset = options.offset;
  while (options.isCurrent()) {
    const pages = await Promise.allSettled(
      Array.from({ length: 4 }, (_, index) =>
        options.loadPage(offset + index * options.pageSize),
      ),
    );
    if (!options.isCurrent()) return null;
    for (const result of pages) {
      if (result.status === "rejected") throw result.reason;
      const page = result.value;
      items.push(...page.items);
      if (!page.hasMore) {
        options.onProgress(options.offset + items.length);
        return items;
      }
      // Offset scheduling requires complete nonterminal pages; never skip rows
      // silently if the server changes its page-size contract.
      if (page.items.length !== options.pageSize)
        throw new Error("Incomplete nonterminal catalog page");
    }
    options.onProgress(options.offset + items.length);
    offset += 4 * options.pageSize;
  }
  return null;
}
