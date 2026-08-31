export type ServiceLogContext = {
  request?: Record<string, unknown>;
  actor?: Record<string, unknown>;
  workflow?: Record<string, unknown>;
  resource?: Record<string, unknown>;
};

export type ParsedServiceLogLine = {
  timestamp: string;
  level: string;
  target: string;
  fields: Record<string, unknown>;
  context: ServiceLogContext | null;
  event: Record<string, unknown>;
  human: string;
};

const LEVELS = new Set(["error", "warn", "info", "debug", "trace"]);

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function formatValue(value: unknown): string {
  if (value === null) return "null";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function contextSummary(context: ServiceLogContext | null): string[] {
  if (!context) return [];
  const values: string[] = [];
  const actor = context.actor;
  const actorName = text(actor?.display_name) ?? text(actor?.id);
  if (actorName) values.push(`actor=${actorName}`);

  const workflow = context.workflow;
  const workflowKind = text(workflow?.kind);
  const workflowId = text(workflow?.id);
  if (workflowKind && workflowId) values.push(`${workflowKind}=${workflowId}`);

  const resource = context.resource;
  for (const key of ["title_id", "import_id", "download_id", "job_id", "client_id", "indexer_id"]) {
    const value = text(resource?.[key]);
    if (value) values.push(`${key}=${value}`);
  }
  return values;
}

function humanLine(
  timestamp: string,
  level: string,
  target: string,
  fields: Record<string, unknown>,
  context: ServiceLogContext | null,
): string {
  const message = formatValue(fields.message ?? "");
  const fieldText = Object.entries(fields)
    .filter(([key]) => key !== "message")
    .map(([key, value]) => `${key}=${formatValue(value)}`);
  const details = [...fieldText, ...contextSummary(context)];
  return `${timestamp} ${level.toUpperCase().padStart(5)} ${target}: ${message}${details.length ? ` ${details.join(" ")}` : ""}`;
}

export function parseServiceLogLine(raw: string): ParsedServiceLogLine | null {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  const event = asRecord(value);
  if (!event) return null;
  const timestamp = text(event.timestamp);
  const level = text(event.level)?.toLowerCase();
  const target = text(event.target);
  const fields = asRecord(event.fields);
  if (!timestamp || !level || !LEVELS.has(level) || !target || !fields) {
    return null;
  }
  const context = asRecord(event.context) as ServiceLogContext | null;
  return {
    timestamp,
    level,
    target,
    fields,
    context,
    event,
    human: humanLine(timestamp, level, target, fields, context),
  };
}

export function prettyServiceLogLine(parsed: ParsedServiceLogLine | null): string | null {
  return parsed ? JSON.stringify(parsed.event, null, 2) : null;
}
