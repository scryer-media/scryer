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

export type GraphQLErrorShape = {
  message?: string;
  extensions?: {
    message?: string;
    details?: string;
    reason?: string;
    error?: string;
    [key: string]: unknown;
  };
  extensionsMessage?: string;
  detail?: string;
};

export type GraphQLResponseShape<T> = {
  data?: T;
  errors?: GraphQLErrorShape[];
};
