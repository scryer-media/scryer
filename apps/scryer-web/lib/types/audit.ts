export type AuditLogEvent = {
  sequence: number;
  eventId: string;
  occurredAt: string;
  actorKind: "USER" | "ANONYMOUS" | "SYSTEM";
  actorUserId: string | null;
  actorDisplayName: string;
  titleId: string | null;
  facet: string | null;
  eventType: string;
  streamKind:
    "GLOBAL" | "TITLE" | "LIBRARY_SCAN" | "JOB_RUN" | "DOWNLOAD_QUEUE_ITEM";
  streamId: string | null;
  payloadJson: unknown;
};
