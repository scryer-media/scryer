import {
  activityChannelValues,
  activityKindValues,
  activitySeverityValues,
} from "@/lib/types/activity";
import type {
  ActivityChannel,
  ActivityEvent,
  ActivityKind,
  ActivitySeverity,
} from "@/lib/types/activity";

const activityKindSet = new Set<string>(activityKindValues);
const activitySeveritySet = new Set<string>(activitySeverityValues);
const activityChannelSet = new Set<string>(activityChannelValues);

function normalizeActivityKind(value: string | undefined): ActivityKind {
  const normalized = (value ?? "").trim().toLowerCase();
  return activityKindSet.has(normalized)
    ? (normalized as ActivityKind)
    : "system_notice";
}

function normalizeActivitySeverity(value: string | undefined): ActivitySeverity {
  const normalized = (value ?? "").trim().toLowerCase();
  return activitySeveritySet.has(normalized)
    ? (normalized as ActivitySeverity)
    : "info";
}

function normalizeActivityChannels(values: unknown): ActivityChannel[] {
  if (!Array.isArray(values)) {
    return ["web_ui", "toast"];
  }

  const normalized = values
    .filter((value): value is string => typeof value === "string")
    .map((value) => value.trim().toLowerCase())
    .filter((value): value is ActivityChannel => activityChannelSet.has(value));

  return normalized.length > 0 ? normalized : ["web_ui", "toast"];
}

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
  const normalizedKind = normalizeActivityKind(input.kind ?? input.eventType);
  const normalizedSeverity = normalizeActivitySeverity(input.severity);
  const normalizedChannels = normalizeActivityChannels(input.channels);

  return {
    id:
      input.id ??
      `activity-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    kind: normalizedKind,
    eventType: input.eventType ?? normalizedKind,
    severity: normalizedSeverity,
    channels: normalizedChannels,
    actorUserId: input.actorUserId ?? null,
    titleId: input.titleId ?? null,
    facet: input.facet ?? null,
    message: input.message ?? "",
    occurredAt: input.occurredAt ?? new Date().toISOString(),
  };
}
