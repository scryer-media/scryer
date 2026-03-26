import type { Facet } from "./titles";

export const activityKindValues = [
  "setting_saved",
  "movie_fetched",
  "movie_added",
  "metadata_hydration_started",
  "metadata_hydration_completed",
  "metadata_hydration_failed",
  "movie_downloaded",
  "series_episode_imported",
  "acquisition_search_completed",
  "acquisition_candidate_accepted",
  "acquisition_candidate_rejected",
  "acquisition_download_failed",
  "post_processing_completed",
  "file_upgraded",
  "import_rejected",
  "subtitle_downloaded",
  "subtitle_search_failed",
  "system_notice",
] as const;

export type ActivityKind = (typeof activityKindValues)[number];

export const activitySeverityValues = [
  "info",
  "success",
  "warning",
  "error",
] as const;

export type ActivitySeverity = (typeof activitySeverityValues)[number];

export const activityChannelValues = ["web_ui", "toast"] as const;

export type ActivityChannel = (typeof activityChannelValues)[number];

export type ActivityEvent = {
  id: string;
  kind: ActivityKind;
  severity: ActivitySeverity;
  channels: ActivityChannel[];
  eventType?: ActivityKind;
  message: string;
  actorUserId?: string | null;
  titleId?: string | null;
  facet?: Facet | null;
  occurredAt?: string | null;
};
