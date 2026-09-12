type MoveHistoryEvent = {
  eventType: string;
  titleId: string;
  sourcePath?: string | null;
  destPath?: string | null;
  dataJson?: unknown;
};

/** Human-readable details for a completed title relocation. */
export function titleMoveHistoryDetails(event: MoveHistoryEvent) {
  if (event.eventType !== "title_moved") return null;
  const data =
    event.dataJson && typeof event.dataJson === "object"
      ? (event.dataJson as Record<string, unknown>)
      : {};
  const text = (key: string) =>
    typeof data[key] === "string" ? (data[key] as string) : "";
  const methods: Record<string, string> = {
    move_with_scryer: "Scryer moved the files",
    user_moved_files: "Files moved manually; catalog updated",
    catalog_only: "Catalog updated",
    files_already_there: "Existing files validated; catalog updated",
  };
  const entries = [
    {
      key: "operation",
      value: text("source_library_id") !== text("destination_library_id")
        ? "Library transfer" : "Root move",
    },
    {
      key: "status",
      value: data.completed_with_warnings ? "Completed with warnings" : "Completed",
    },
    { key: "method", value: methods[text("mode")] ?? "" },
    { key: "source_library", value: text("source_library_name") },
    { key: "destination_library", value: text("destination_library_name") },
    { key: "source_path", value: event.sourcePath || text("source_path") },
    { key: "dest_path", value: event.destPath || text("destination_path") },
    {
      key: "merged_from",
      value: text("source_title_id") && text("source_title_id") !== event.titleId
        ? text("source_title_name") : "",
    },
    { key: "notes", value: text("detail") },
  ];
  return entries.filter((entry) => entry.value);
}
