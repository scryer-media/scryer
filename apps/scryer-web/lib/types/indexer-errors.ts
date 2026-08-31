export type IndexerErrorOperation =
  | "CONNECTION_TEST"
  | "INTERACTIVE_SEARCH"
  | "AUTOMATIC_SEARCH"
  | "RSS_SYNC"
  | "INDEXER_ACTION"
  | "MANAGEMENT_SYNC"
  | "CAPS_REFRESH";

export type IndexerErrorSummary = {
  id: string;
  indexerId: string;
  indexerName: string;
  operation: IndexerErrorOperation;
  occurredAt: string;
  httpStatus: number | null;
  classification: string;
  providerErrorCode: number | null;
  message: string;
  contentType: string | null;
};

export type IndexerErrorHeader = {
  name: string;
  valueBase64: string;
  value: string | null;
};

export type IndexerErrorResponse = {
  status: number;
  headers: IndexerErrorHeader[];
  bodyBase64: string;
};

export type IndexerErrorDetail = {
  error: IndexerErrorSummary;
  response: IndexerErrorResponse | null;
};

export type IndexerErrorConnection = {
  items: IndexerErrorSummary[];
  nextCursor: string | null;
};
