import type { IndexerProxyDraft, UiDateTimeFormat } from "../types/index.ts";

function buildIndexerProxyCommonInput(draft: IndexerProxyDraft) {
  return {
    name: draft.name.trim(),
    baseUrl: draft.baseUrl.trim(),
    requestTimeoutSeconds: draft.requestTimeoutSeconds,
    isEnabled: draft.isEnabled,
  };
}

export function buildCreateIndexerProxyInput(draft: IndexerProxyDraft) {
  return {
    providerType: draft.providerType,
    ...buildIndexerProxyCommonInput(draft),
  };
}

export function buildUpdateIndexerProxyInput(
  id: string,
  draft: IndexerProxyDraft,
) {
  return {
    id,
    ...buildIndexerProxyCommonInput(draft),
  };
}

export function parseUiDateTimeFormat(value: string): UiDateTimeFormat | null {
  return value === "LOCALE" || value === "ISO24H" ? value : null;
}

export function buildDownloadClientConnectionTestInput<TConfig>(
  id: string | null,
  clientType: string,
  config: TConfig,
) {
  const common = { clientType, config };
  return id ? { id, ...common } : common;
}
