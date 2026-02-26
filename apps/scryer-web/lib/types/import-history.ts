export type ImportRecord = {
  id: string;
  sourceSystem: string;
  sourceRef: string;
  sourceTitle: string | null;
  importType: string;
  status: string;
  errorMessage: string | null;
  decision: string | null;
  skipReason: string | null;
  titleId: string | null;
  sourcePath: string | null;
  destPath: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  createdAt: string;
};
