import { isResolvableConflict } from "./pending-import-conflicts.ts";

type SelectableImport = {
  id: string;
  status: string;
  facet: string;
  reason: string;
};

export function resolvablePendingImportSelection<T extends SelectableImport>(
  items: readonly T[],
): T[] {
  return items.filter(isResolvableConflict);
}

export function canResolvePendingImportSelection(
  items: readonly SelectableImport[],
): boolean {
  return items.some(isResolvableConflict);
}

export function togglePendingImportSelection<T extends SelectableImport>(
  current: readonly T[],
  items: readonly T[],
  checked: boolean,
): T[] {
  const ids = new Set(items.map((item) => item.id));
  const remaining = current.filter((item) => !ids.has(item.id));
  return checked
    ? [
        ...remaining,
        ...new Map(
          items
            .filter((item) => item.status === "PENDING")
            .map((item) => [item.id, item]),
        ).values(),
      ]
    : remaining;
}

/** Ids per ignore request; a larger selection is sent in several. */
export const PENDING_IMPORT_IGNORE_CHUNK = 500;

export type PendingImportIgnoreChunkResult = {
  ignoredIds: readonly string[];
  failed: readonly { id: string; error: unknown }[];
};

export async function ignorePendingImportBatch<
  T extends { id: string; status: string },
>(
  items: readonly T[],
  ignore: (ids: string[]) => Promise<PendingImportIgnoreChunkResult>,
  chunkSize: number = PENDING_IMPORT_IGNORE_CHUNK,
): Promise<{ ignored: T[]; failed: { item: T; error: unknown }[] }> {
  const ignored: T[] = [];
  const failed: { item: T; error: unknown }[] = [];
  const unique = [
    ...new Map(
      items
        .filter((item) => item.status === "PENDING")
        .map((item) => [item.id, item]),
    ).values(),
  ];
  const size = Math.max(1, Math.floor(chunkSize));
  for (let start = 0; start < unique.length; start += size) {
    const chunk = unique.slice(start, start + size);
    let outcome: PendingImportIgnoreChunkResult;
    try {
      outcome = await ignore(chunk.map((item) => item.id));
    } catch (error) {
      for (const item of chunk) failed.push({ item, error });
      continue;
    }
    const ignoredIds = new Set(outcome.ignoredIds);
    const errors = new Map(outcome.failed.map(({ id, error }) => [id, error]));
    for (const item of chunk) {
      if (ignoredIds.has(item.id)) ignored.push(item);
      // An id the server did not answer has no error of its own; the
      // caller shows its generic failure text for it.
      else failed.push({ item, error: errors.get(item.id) });
    }
  }
  return { ignored, failed };
}
