import { useState } from "react";
import type { TitleRecord, LibraryScanSummary } from "@/lib/types";

export function useTitleManagementState() {
  const [titleFilter, setTitleFilter] = useState("");
  const [monitoredTitles, setMonitoredTitles] = useState<TitleRecord[]>([]);
  const [titleLoading, setTitleLoading] = useState(false);
  const [titleStatus, setTitleStatus] = useState("");
  const [titleToDelete, setTitleToDelete] = useState<TitleRecord | null>(null);
  const [deleteFilesOnDisk, setDeleteFilesOnDisk] = useState(false);
  const [deleteTitleLoadingById, setDeleteTitleLoadingById] = useState<
    Record<string, boolean>
  >({});
  const [libraryScanLoading, setLibraryScanLoading] = useState(false);
  const [libraryScanSummary, setLibraryScanSummary] = useState<LibraryScanSummary | null>(null);

  return {
    titleFilter,
    setTitleFilter,
    monitoredTitles,
    setMonitoredTitles,
    titleLoading,
    setTitleLoading,
    titleStatus,
    setTitleStatus,
    titleToDelete,
    setTitleToDelete,
    deleteFilesOnDisk,
    setDeleteFilesOnDisk,
    deleteTitleLoadingById,
    setDeleteTitleLoadingById,
    libraryScanLoading,
    setLibraryScanLoading,
    libraryScanSummary,
    setLibraryScanSummary,
  };
}
