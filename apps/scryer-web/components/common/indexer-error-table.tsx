import * as React from "react";
import {
  ChevronDown,
  ChevronUp,
  Download,
  FileWarning,
  Loader2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableCodeCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { LazyCodeEditor } from "@/components/common/lazy-code-editor";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type {
  IndexerErrorDetail,
  IndexerErrorResponse,
  IndexerErrorSummary,
} from "@/lib/types";
import { formatUiDate, formatUiTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  decodeIndexerErrorBody,
  indexerErrorDownloadExtension,
  isSensitiveIndexerErrorHeader,
  presentIndexerErrorBody,
} from "@/lib/utils/indexer-error-response";

function humanizeEnum(value: string): string {
  return value
    .toLowerCase()
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function formatByteCount(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}

function downloadResponseBody(
  detail: IndexerErrorDetail,
  response: IndexerErrorResponse,
) {
  const presentation = presentIndexerErrorBody(
    response.bodyBase64,
    detail.error.contentType,
  );
  const bytes = decodeIndexerErrorBody(response.bodyBase64);
  const blob = new Blob([Uint8Array.from(bytes).buffer], {
    type: detail.error.contentType ?? "application/octet-stream",
  });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = `indexer-error-${detail.error.id}.${indexerErrorDownloadExtension(presentation.format)}`;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
}

function IndexerErrorResponseDetail({ detail }: { detail: IndexerErrorDetail }) {
  const t = useTranslate();
  if (detail.response == null) {
    return (
      <div className="rounded-lg border border-border/60 bg-background/60 p-4 text-sm text-muted-foreground">
        {t("indexerErrors.noHttpResponse")}
      </div>
    );
  }

  return <IndexerHttpResponseDetail detail={detail} response={detail.response} />;
}

function IndexerHttpResponseDetail({
  detail,
  response,
}: {
  detail: IndexerErrorDetail;
  response: IndexerErrorResponse;
}) {
  const t = useTranslate();
  const presentation = React.useMemo(
    () => presentIndexerErrorBody(response.bodyBase64, detail.error.contentType),
    [detail.error.contentType, response.bodyBase64],
  );
  const [showFormatted, setShowFormatted] = React.useState(
    presentation.formattedText != null,
  );
  const displayedText = showFormatted && presentation.formattedText != null
    ? presentation.formattedText
    : presentation.rawText;
  const editorLanguage = presentation.format === "json"
    ? "javascript"
    : presentation.format === "xml"
      ? "xml"
      : "plain";

  return (
    <div className="space-y-4 rounded-lg border border-border/60 bg-background/60 p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h4 className="text-sm font-semibold text-foreground">
            {t("indexerErrors.response")}
          </h4>
          <p className="mt-1 text-xs text-muted-foreground">
            HTTP {response.status} · {formatByteCount(presentation.byteLength)}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          variant="secondary"
          className="gap-2"
          onClick={() => downloadResponseBody(detail, response)}
        >
          <Download className="h-4 w-4" />
          {t("indexerErrors.downloadFullBody")}
        </Button>
      </div>

      <div>
        <h5 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {t("indexerErrors.headers")}
        </h5>
        {response.headers.length === 0 ? (
          <p className="text-xs text-muted-foreground">{t("indexerErrors.noHeaders")}</p>
        ) : (
          <div className="overflow-hidden rounded-md border border-border/60">
            {response.headers.map((header, index) => {
              const sensitive = isSensitiveIndexerErrorHeader(header.name);
              return (
                <div
                  key={`${header.name}-${index}`}
                  className="grid gap-1 border-b border-border/50 px-3 py-2 text-xs last:border-b-0 sm:grid-cols-[minmax(10rem,14rem)_1fr]"
                >
                  <span className="break-all font-mono font-medium text-foreground">
                    {header.name}
                  </span>
                  <span className="break-all font-mono text-muted-foreground">
                    {sensitive
                      ? t("indexerErrors.redacted")
                      : header.value ?? `${t("indexerErrors.base64Value")}: ${header.valueBase64}`}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>

      <div>
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <h5 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {t("indexerErrors.body")}
          </h5>
          {presentation.formattedText != null ? (
            <div className="flex rounded-md border border-border/60 p-0.5">
              <Button
                type="button"
                size="sm"
                variant={showFormatted ? "secondary" : "ghost"}
                className="h-7 px-2 text-xs"
                onClick={() => setShowFormatted(true)}
              >
                {t("indexerErrors.formatted")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant={!showFormatted ? "secondary" : "ghost"}
                className="h-7 px-2 text-xs"
                onClick={() => setShowFormatted(false)}
              >
                {t("indexerErrors.raw")}
              </Button>
            </div>
          ) : null}
        </div>
        {presentation.truncated ? (
          <p className="mb-2 text-xs text-muted-foreground">
            {t("indexerErrors.previewTruncated", { size: formatByteCount(presentation.byteLength) })}
          </p>
        ) : null}
        {displayedText != null ? (
          <LazyCodeEditor
            id={selectorId("indexer-error-body", detail.error.id)}
            value={displayedText}
            onChange={() => undefined}
            readOnly
            language={editorLanguage}
            height="360px"
          />
        ) : presentation.byteLength === 0 ? (
          <p className="text-sm text-muted-foreground">{t("indexerErrors.emptyBody")}</p>
        ) : (
          <div className="flex items-center gap-2 rounded-md border border-border/60 p-3 text-sm text-muted-foreground">
            <FileWarning className="h-4 w-4" />
            {t("indexerErrors.binaryBody", { size: formatByteCount(presentation.byteLength) })}
          </div>
        )}
      </div>
    </div>
  );
}

export function IndexerErrorTable({
  items,
  showIndexer = false,
  details,
  detailLoading,
  detailErrors,
  onToggleDetail,
  emptyMessage,
}: {
  items: IndexerErrorSummary[];
  showIndexer?: boolean;
  details: Record<string, IndexerErrorDetail | undefined>;
  detailLoading: Record<string, boolean | undefined>;
  detailErrors: Record<string, string | undefined>;
  onToggleDetail: (id: string, expanded: boolean) => void;
  emptyMessage: string;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [expandedRowId, setExpandedRowId] = React.useState<string | null>(null);
  const columnCount = showIndexer ? 8 : 7;

  const toggleExpanded = React.useCallback((id: string) => {
    const expanded = expandedRowId !== id;
    setExpandedRowId(expanded ? id : null);
    onToggleDetail(id, expanded);
  }, [expandedRowId, onToggleDetail]);

  if (items.length === 0) {
    return <p className="py-8 text-sm text-muted-foreground">{emptyMessage}</p>;
  }

  return (
    <div className="overflow-hidden">
      <Table overflow="clip" layout="fixed" density="dense">
        <TableHeader>
          <TableRow>
            <TableHead className="w-10 text-center" />
            <TableHead className="w-36">{t("indexerErrors.occurred")}</TableHead>
            {showIndexer ? <TableHead className="w-48">{t("indexerErrors.indexer")}</TableHead> : null}
            <TableHead className="w-40">{t("indexerErrors.operation")}</TableHead>
            <TableHead className="w-20 text-center">{t("indexerErrors.http")}</TableHead>
            <TableHead className="w-48">{t("indexerErrors.classification")}</TableHead>
            <TableHead>{t("indexerErrors.message")}</TableHead>
            <TableHead className="w-40">{t("indexerErrors.contentType")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {items.map((item) => {
            const expanded = expandedRowId === item.id;
            const detail = details[item.id];
            return (
              <React.Fragment key={item.id}>
                <TableRow
                  id={selectorId("indexer-error-row", item.id)}
                  data-ui="settings-table-row"
                >
                  <TableCell className="text-center">
                    <button
                      type="button"
                      className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border/60 bg-card/80 text-muted-foreground transition hover:text-foreground"
                      onClick={() => toggleExpanded(item.id)}
                      aria-expanded={expanded}
                      aria-controls={selectorId("indexer-error-detail", item.id)}
                      aria-label={expanded ? t("indexerErrors.collapse") : t("indexerErrors.expand")}
                    >
                      {expanded ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
                    </button>
                  </TableCell>
                  <TableCell className="align-top text-sm">
                    <div className="font-medium text-foreground">
                      {formatUiDate(item.occurredAt, dateTimeFormat)}
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">
                      {formatUiTime(item.occurredAt, dateTimeFormat)}
                    </div>
                  </TableCell>
                  {showIndexer ? (
                    <TableCell className="align-top">
                      <div className="truncate text-sm font-medium" title={item.indexerName}>
                        {item.indexerName}
                      </div>
                    </TableCell>
                  ) : null}
                  <TableCell className="align-top text-sm">{humanizeEnum(item.operation)}</TableCell>
                  <TableCodeCell className="align-top text-center text-sm">
                    {item.httpStatus ?? "—"}
                  </TableCodeCell>
                  <TableCell className="align-top text-sm">
                    <div>{humanizeEnum(item.classification)}</div>
                    {item.providerErrorCode != null ? (
                      <div className="mt-1 text-xs text-muted-foreground">
                        {t("indexerErrors.providerCode", { code: item.providerErrorCode })}
                      </div>
                    ) : null}
                  </TableCell>
                  <TableCell className="align-top">
                    <div className="truncate text-sm" title={item.message}>
                      {item.message}
                    </div>
                  </TableCell>
                  <TableCell className="align-top">
                    <div
                      className="truncate font-mono text-xs text-muted-foreground"
                      title={item.contentType ?? undefined}
                    >
                      {item.contentType ?? "—"}
                    </div>
                  </TableCell>
                </TableRow>
                {expanded ? (
                  <TableRow id={selectorId("indexer-error-detail", item.id)}>
                    <TableCell colSpan={columnCount} className="bg-card/30 p-4">
                      {detailLoading[item.id] ? (
                        <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
                          <Loader2 className="h-4 w-4 animate-spin" />
                          {t("indexerErrors.loadingResponse")}
                        </div>
                      ) : detailErrors[item.id] ? (
                        <div role="alert" className="flex flex-wrap items-center gap-3 text-sm text-[var(--scry-danger-text)]">
                          <span className="min-w-0 flex-1">{detailErrors[item.id]}</span>
                          <Button type="button" size="sm" variant="secondary" onClick={() => onToggleDetail(item.id, true)}>
                            {t("indexerErrors.retry")}
                          </Button>
                        </div>
                      ) : detail ? (
                        <IndexerErrorResponseDetail detail={detail} />
                      ) : null}
                    </TableCell>
                  </TableRow>
                ) : null}
              </React.Fragment>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}
