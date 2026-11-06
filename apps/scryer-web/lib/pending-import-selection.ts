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

export async function ignorePendingImportBatch<
  T extends { id: string; status: string },
>(
  items: readonly T[],
  ignore: (id: string) => Promise<void>,
): Promise<{ ignored: T[]; failed: { item: T; error: unknown }[] }> {
  const ignored: T[] = [];
  const failed: { item: T; error: unknown }[] = [];
  const unique = new Map(
    items
      .filter((item) => item.status === "PENDING")
      .map((item) => [item.id, item]),
  );
  for (const item of unique.values()) {
    try {
      await ignore(item.id);
      ignored.push(item);
    } catch (error) {
      failed.push({ item, error });
    }
  }
  return { ignored, failed };
}
