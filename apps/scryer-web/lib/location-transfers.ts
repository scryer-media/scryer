import type {
  LocationOperation,
  LocationTitleCheckpointState,
  LongValue,
} from "./location-operations";

export function transferArtworkRequest(titleIds: string[]) {
  const ids = [...new Set(titleIds)].slice(0, 50).sort();
  return {
    query: ids.length
      ? `query TransferTitleArtwork(${ids.map((_, i) => `$id${i}: ID!`).join(", ")}) {
          ${ids.map((_, i) => `title${i}: title(id: $id${i}) { id posterUrl }`).join("\n")}
        }`
      : "query TransferTitleArtwork { __typename }",
    variables: Object.fromEntries(ids.map((id, i) => [`id${i}`, id])),
  };
}

export type TransferTitle = {
  titleId: string;
  name: string;
  state: LocationTitleCheckpointState;
  filesTotal: LongValue;
  filesDone: LongValue;
  bytesTotal: LongValue;
  copyBytes: LongValue;
  verificationBytes: LongValue;
  copying: number;
  verifying: number;
  currentFile: string | null;
  hasException: boolean;
};
export type TransferSnapshot = {
  generation: LongValue;
  revision: LongValue;
  operation: LocationOperation;
  progressBasisPoints: number;
  etaSeconds: LongValue | null;
  titles: TransferTitle[];
  totalCount: LongValue;
  hasMore: boolean;
};
export type TransferScope = {
  operationId: string;
  page: number | null;
  requestGeneration: number;
};
export type TransferView = {
  scope: TransferScope;
  snapshot: TransferSnapshot | null;
};

export function acceptTransferSnapshot(
  current: TransferView,
  scope: TransferScope,
  incoming: TransferSnapshot,
): TransferView {
  if (
    current.scope.operationId !== scope.operationId ||
    current.scope.page !== scope.page ||
    current.scope.requestGeneration !== scope.requestGeneration ||
    incoming.operation.id !== scope.operationId
  )
    return current;
  const previous = current.snapshot;
  if (previous) {
    const generation =
      BigInt(incoming.generation) - BigInt(previous.generation);
    if (
      generation < 0n ||
      (generation === 0n &&
        BigInt(incoming.revision) <= BigInt(previous.revision))
    )
      return current;
  }
  return {
    scope,
    snapshot: {
      ...incoming,
      progressBasisPoints: Math.max(
        previous?.progressBasisPoints ?? 0,
        incoming.progressBasisPoints,
      ),
    },
  };
}

/** File work can finish before catalog finalization. Never round that tail up. */
export function transferOperationProgress(
  snapshot: Pick<TransferSnapshot, "operation" | "progressBasisPoints">,
): number {
  if (
    ["COMPLETED", "COMPLETED_WITH_WARNINGS"].includes(snapshot.operation.state)
  )
    return 100;
  return (
    Math.floor(Math.max(0, Math.min(9999, snapshot.progressBasisPoints)) / 10) /
    10
  );
}

export function transferTitleProgress(row: TransferTitle): number {
  if (["COMPLETED", "COMPLETED_WITH_WARNINGS", "SKIPPED"].includes(row.state))
    return 100;
  const total = Number(row.bytesTotal);
  return total > 0
    ? Math.min(
        99.9,
        ((Number(row.copyBytes) + Number(row.verificationBytes)) /
          (2 * total)) *
          100,
      )
    : 0;
}

export function splitTransferPath(path: string): {
  directory: string;
  filename: string;
} {
  const index = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return {
    directory: path.slice(0, index + 1),
    filename: path.slice(index + 1),
  };
}
