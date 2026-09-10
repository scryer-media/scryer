import type {
  LocationOperation,
  LocationTitleCheckpointState,
  LongValue,
} from "./location-operations";

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

export function transferTitleProgress(row: TransferTitle): number {
  const total = Number(row.bytesTotal);
  return total > 0
    ? Math.min(
        100,
        ((Number(row.copyBytes) + Number(row.verificationBytes)) /
          (2 * total)) *
          100,
      )
    : ["COMPLETED", "COMPLETED_WITH_WARNINGS", "SKIPPED"].includes(row.state)
      ? 100
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
