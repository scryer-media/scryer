
import { useState } from "react";
import { RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { ImportRecord } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";

type ImportHistoryViewProps = {
  records: ImportRecord[];
  loading: boolean;
  error: string | null;
  limit: number;
  onLimitChange: (limit: number) => void;
  onRefresh: () => void;
};

const statusClasses: Record<string, string> = {
  completed:
    "border-emerald-500/40 bg-emerald-500/15 dark:bg-emerald-500/10 text-emerald-700 dark:text-emerald-200",
  failed: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  skipped: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  processing: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  queued: "border-border/40 bg-muted-foreground/10 text-card-foreground",
};

type StatusFilter = "all" | "completed" | "failed" | "skipped";

function formatTimestamp(ts: string | null): string {
  if (!ts) return "\u2014";
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: "short",
      timeStyle: "short",
    }).format(new Date(ts));
  } catch {
    return ts;
  }
}

export function ImportHistoryView({
  records,
  loading,
  error,
  limit,
  onLimitChange,
  onRefresh,
}: ImportHistoryViewProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");

  const filtered =
    statusFilter === "all"
      ? records
      : records.filter((r) => r.status === statusFilter);

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-center gap-3">
            <CardTitle>{t("importHistory.title")}</CardTitle>
            <span className="text-sm text-muted-foreground">
              ({filtered.length}{filtered.length !== records.length ? ` / ${records.length}` : ""})
            </span>
          </div>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <Select
              value={statusFilter}
              onValueChange={(v) => setStatusFilter(v as StatusFilter)}
            >
              <SelectTrigger className="h-9 w-full sm:w-36">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">All statuses</SelectItem>
                <SelectItem value="completed">Completed</SelectItem>
                <SelectItem value="failed">Failed</SelectItem>
                <SelectItem value="skipped">Skipped</SelectItem>
              </SelectContent>
            </Select>
            <Select
              value={String(limit)}
              onValueChange={(v) => onLimitChange(Number(v))}
            >
              <SelectTrigger className="h-9 w-full sm:w-28">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="50">50 records</SelectItem>
                <SelectItem value="100">100 records</SelectItem>
                <SelectItem value="250">250 records</SelectItem>
                <SelectItem value="500">500 records</SelectItem>
              </SelectContent>
            </Select>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              className="w-full sm:w-auto"
              disabled={loading}
              onClick={onRefresh}
            >
              <RefreshCw className="mr-2 h-4 w-4" />
              {loading ? t("label.refreshing") : t("label.refresh")}
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent>
        {error ? (
          <p className="rounded border border-rose-500/40 bg-rose-950/40 p-2 text-sm text-rose-200">
            {error}
          </p>
        ) : loading && records.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("label.loading")}</p>
        ) : filtered.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("importHistory.empty")}</p>
        ) : isMobile ? (
          <div className="space-y-3">
            {filtered.map((record) => {
              const hasPaths = record.sourcePath || record.destPath;
              return (
                <div key={record.id} className="rounded-xl border border-border bg-card/30 p-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p
                        className="break-words text-sm font-medium text-foreground"
                        title={record.sourceTitle ?? record.sourceRef}
                      >
                        {record.sourceTitle ?? record.sourceRef}
                      </p>
                      {record.sourceTitle ? (
                        <p
                          className="mt-1 break-words text-xs text-muted-foreground"
                          title={record.sourceRef}
                        >
                          {record.sourceRef}
                        </p>
                      ) : null}
                    </div>
                    <span
                      className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${statusClasses[record.status] ?? "border-border bg-muted text-card-foreground"}`}
                    >
                      {record.status}
                    </span>
                  </div>
                  <div className="mt-2 flex flex-wrap gap-2 text-xs text-muted-foreground">
                    <span>{record.importType}</span>
                    <span>{formatTimestamp(record.createdAt)}</span>
                    {record.finishedAt && record.finishedAt !== record.createdAt ? (
                      <span>{formatTimestamp(record.finishedAt)}</span>
                    ) : null}
                  </div>
                  {record.decision ? (
                    <p className="mt-2 text-xs text-foreground">{record.decision}</p>
                  ) : null}
                  {record.skipReason ? (
                    <p className="mt-1 break-words text-xs text-muted-foreground">
                      {record.skipReason}
                    </p>
                  ) : null}
                  {record.errorMessage ? (
                    <p className="mt-2 break-words text-xs text-rose-400">{record.errorMessage}</p>
                  ) : null}
                  {hasPaths ? (
                    <div className="mt-2 space-y-1 text-xs text-muted-foreground">
                      {record.sourcePath ? (
                        <p className="break-all">
                          <span className="font-medium text-foreground/80">From:</span>{" "}
                          {record.sourcePath}
                        </p>
                      ) : null}
                      {record.destPath ? (
                        <p className="break-all">
                          <span className="font-medium text-foreground/80">To:</span>{" "}
                          {record.destPath}
                        </p>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <Table className="table-fixed min-w-[860px]">
              <TableHeader>
                <TableRow>
                  <TableHead className="w-28 min-w-28">{t("importHistory.status")}</TableHead>
                  <TableHead className="w-[30%] min-w-0">{t("importHistory.sourceRef")}</TableHead>
                  <TableHead className="w-28 min-w-0">Type</TableHead>
                  <TableHead className="w-[18%] min-w-0">{t("importHistory.decision")}</TableHead>
                  <TableHead className="w-[20%] min-w-0">{t("importHistory.error")}</TableHead>
                  <TableHead className="w-36 min-w-36">{t("importHistory.createdAt")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filtered.map((record) => {
                  const hasError = Boolean(record.errorMessage);
                  const hasPaths = record.sourcePath || record.destPath;

                  return (
                    <TableRow key={record.id}>
                      {/* Status */}
                      <TableCell className="align-middle">
                        <span
                          className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${statusClasses[record.status] ?? "border-border bg-muted text-card-foreground"}`}
                        >
                          {record.status}
                        </span>
                      </TableCell>

                      {/* Source */}
                      <TableCell className="min-w-0 align-middle">
                        <p
                          className="break-words whitespace-normal text-sm"
                          title={record.sourceTitle ?? record.sourceRef}
                        >
                          {record.sourceTitle ?? record.sourceRef}
                        </p>
                        {record.sourceTitle ? (
                          <p
                            className="text-xs text-muted-foreground break-words whitespace-normal"
                            title={record.sourceRef}
                          >
                            {record.sourceRef}
                          </p>
                        ) : null}
                        {hasPaths ? (
                          <HoverCard openDelay={200} closeDelay={100}>
                            <HoverCardTrigger asChild>
                              <button
                                type="button"
                                className="mt-0.5 text-xs text-muted-foreground/70 underline decoration-dotted hover:text-muted-foreground"
                              >
                                paths
                              </button>
                            </HoverCardTrigger>
                            <HoverCardContent sideOffset={4} className="w-96 max-w-sm text-xs">
                              {record.sourcePath ? (
                                <div className="mb-1">
                                  <span className="font-medium text-muted-foreground">From: </span>
                                  <span className="break-all">{record.sourcePath}</span>
                                </div>
                              ) : null}
                              {record.destPath ? (
                                <div>
                                  <span className="font-medium text-muted-foreground">To: </span>
                                  <span className="break-all">{record.destPath}</span>
                                </div>
                              ) : null}
                            </HoverCardContent>
                          </HoverCard>
                        ) : null}
                      </TableCell>

                      {/* Type */}
                      <TableCell className="min-w-0 align-middle">
                        <span className="text-xs text-muted-foreground">{record.importType}</span>
                      </TableCell>

                      {/* Decision */}
                      <TableCell className="min-w-0 align-middle">
                        {record.decision ? (
                          <span className="text-xs">{record.decision}</span>
                        ) : null}
                        {record.skipReason ? (
                          <p className="text-xs text-muted-foreground break-words whitespace-normal">
                            {record.skipReason}
                          </p>
                        ) : null}
                        {!record.decision && !record.skipReason ? (
                          <span className="text-xs text-muted-foreground/60">{"\u2014"}</span>
                        ) : null}
                      </TableCell>

                      {/* Error */}
                      <TableCell className="min-w-0 align-middle">
                        {hasError ? (
                          <HoverCard openDelay={200} closeDelay={100}>
                            <HoverCardTrigger asChild>
                              <p className="max-w-full cursor-default break-words whitespace-normal text-xs text-rose-400 line-clamp-2">
                                {record.errorMessage}
                              </p>
                            </HoverCardTrigger>
                            <HoverCardContent sideOffset={4} className="max-w-sm text-xs">
                              <p className="whitespace-pre-wrap break-words text-foreground">
                                {record.errorMessage}
                              </p>
                            </HoverCardContent>
                          </HoverCard>
                        ) : (
                          <span className="text-xs text-muted-foreground/60">{"\u2014"}</span>
                        )}
                      </TableCell>

                      {/* Date */}
                      <TableCell className="align-middle">
                        <p className="text-xs text-muted-foreground">
                          {formatTimestamp(record.createdAt)}
                        </p>
                        {record.finishedAt && record.finishedAt !== record.createdAt ? (
                          <p className="text-xs text-muted-foreground/60" title="Finished">
                            {formatTimestamp(record.finishedAt)}
                          </p>
                        ) : null}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
