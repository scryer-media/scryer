/**
 * Row selection in the Manual Import dialog: which files a bulk action works
 * on. Selection never decides what is queued; a row is queued when it has a
 * target, as before.
 */

/** The dialog's rows, in list order. */
type Rows = readonly { candidateId: string; fileName: string }[];

/** `selected` with `candidateId` added, or removed when it is already in. */
export function toggledRowSelection(
  selected: ReadonlySet<string>,
  candidateId: string,
): Set<string> {
  const next = new Set(selected);
  if (next.has(candidateId)) {
    next.delete(candidateId);
  } else {
    next.add(candidateId);
  }
  return next;
}

/** Every listed row. */
export function allRowsSelected(rows: Rows): Set<string> {
  return new Set(rows.map((row) => row.candidateId));
}

/** The listed rows that are not selected now. */
export function invertedRowSelection(
  rows: Rows,
  selected: ReadonlySet<string>,
): Set<string> {
  return new Set(
    rows
      .map((row) => row.candidateId)
      .filter((candidateId) => !selected.has(candidateId)),
  );
}

/**
 * `selected` plus the rows whose file name contains `text`, ignoring case.
 * Blank text selects nothing more.
 */
export function withRowsMatching(
  rows: Rows,
  selected: ReadonlySet<string>,
  text: string,
): Set<string> {
  const needle = text.trim().toLocaleLowerCase();
  const next = new Set(selected);
  if (!needle) {
    return next;
  }
  for (const row of rows) {
    if (row.fileName.toLocaleLowerCase().includes(needle)) {
      next.add(row.candidateId);
    }
  }
  return next;
}

/**
 * The rows a bulk action works on: the selected rows when any are selected,
 * otherwise every row.
 */
export function rowsForBulkAction<Row extends { candidateId: string }>(
  rows: readonly Row[],
  selected: ReadonlySet<string>,
): Row[] {
  if (selected.size === 0) {
    return [...rows];
  }
  return rows.filter((row) => selected.has(row.candidateId));
}

/**
 * `mappings` with each of `rows` set to `targetOf(row)`. A row for which
 * `targetOf` returns `null` keeps its target. Returns the same object when
 * nothing changes.
 */
export function withRowTargets<Row extends { candidateId: string }>(
  mappings: Readonly<Record<string, string>>,
  rows: readonly Row[],
  targetOf: (row: Row) => string | null,
): Record<string, string> {
  let next: Record<string, string> | null = null;
  for (const row of rows) {
    const target = targetOf(row);
    if (target === null || mappings[row.candidateId] === target) {
      continue;
    }
    next ??= { ...mappings };
    next[row.candidateId] = target;
  }
  return next ?? (mappings as Record<string, string>);
}
