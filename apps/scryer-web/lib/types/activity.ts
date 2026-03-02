export type ActivityEvent = {
  id: string;
  kind: string;
  severity: string;
  channels: string[];
  eventType?: string;
  message: string;
  actorUserId?: string | null;
  titleId?: string | null;
  occurredAt?: string | null;
};

