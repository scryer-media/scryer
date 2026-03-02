import type { ActivityEvent } from "@/lib/types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function toActivityEventCandidates(input: unknown): unknown[] {
  if (Array.isArray(input)) {
    return input;
  }

  if (isActivityEventShape(input)) {
    return [input];
  }

  return [];
}

function isActivityEventShape(value: unknown): value is Record<string, unknown> {
  if (!isRecord(value)) {
    return false;
  }

  const candidate = value as {
    id?: unknown;
    kind?: unknown;
    eventType?: unknown;
    message?: unknown;
  };

  return (
    typeof candidate.id === "string" ||
    typeof candidate.kind === "string" ||
    typeof candidate.eventType === "string" ||
    typeof candidate.message === "string"
  );
}

export function collectActivityEventsFromPayload(payload: unknown): unknown[] {
  if (Array.isArray(payload)) {
    return toActivityEventCandidates(payload);
  }

  if (!isRecord(payload)) {
    return [];
  }

  const directEvents = toActivityEventCandidates(payload.activityEvents);
  if (directEvents.length > 0) {
    return directEvents;
  }

  const eventCandidates = toActivityEventCandidates(payload.event);
  if (eventCandidates.length > 0) {
    return eventCandidates;
  }

  const eventsCandidates = toActivityEventCandidates(payload.events);
  if (eventsCandidates.length > 0) {
    return eventsCandidates;
  }

  if (isRecord(payload.data)) {
    const fromData = collectActivityEventsFromPayload(payload.data);
    if (fromData.length > 0) {
      return fromData;
    }
  }

  if (Array.isArray(payload.data)) {
    const fromData = collectActivityEventsFromPayload(payload.data);
    if (fromData.length > 0) {
      return fromData;
    }
  }

  if (isActivityEventShape(payload)) {
    return [payload];
  }

  return [];
}

export function normalizeActivityEvent(input: Partial<ActivityEvent>): ActivityEvent {
  const normalizedKind = input.kind ?? input.eventType ?? "system_notice";
  const rawChannels = Array.isArray(input.channels) ? input.channels : [];
  const normalizedChannels =
    rawChannels.filter((value) => value.trim().length > 0).map((value) => value.toLowerCase()) ??
    ["web_ui", "toast"];

  return {
    id:
      input.id ??
      `activity-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    kind: normalizedKind,
    eventType: input.eventType ?? normalizedKind,
    severity: input.severity ?? "info",
    channels: normalizedChannels.length ? normalizedChannels : ["web_ui", "toast"],
    actorUserId: input.actorUserId ?? null,
    titleId: input.titleId ?? null,
    message: input.message ?? "",
    occurredAt: input.occurredAt ?? new Date().toISOString(),
  };
}

