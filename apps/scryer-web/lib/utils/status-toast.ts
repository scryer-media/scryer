export type StatusToastKind = "success" | "error" | "warning";

const NO_TOAST_PATTERNS: RegExp[] = [
  /^ready\.?$/i,
  /\bsearching\b/i,
  /\bsearching\s+(?:tvdb|nzb)\b/i,
  /\bfound\s+\d+\s+(?:tvdb|nzb)\b/i,
  /\bfound\s+\d+\s+nzb\s+item/i,
  /\bselected tvdb match/i,
  /\btvdb queue tip/i,
  /\bnzb queue tip/i,
  /\bno nzb results?\b/i,
  /\bno results found\b/i,
  /\bshowing activity stream\b/i,
  /\bhiding activity stream\b/i,
  /\bediting\s+(?:user|indexer|download client)\b/i,
  /\bdelete\s+(?:user|indexer|download client)\?/i,
  /\blanguage set to/i,
  /\bfacet is required\b/i,
  /\btitle is required\b/i,
  /\busername and password are required\b/i,
  /\bpassword is required\b/i,
  /\blibrary scan running\b/i,
  /\btesting nzbget connection/i,
];

const ERROR_PATTERNS: RegExp[] = [
  /\bfailed to\b/i,
  /\bfailed\b/i,
  /\brequest failed\b/i,
  /\berror\b/i,
  /\bfailed to (?:load|save|update|create|delete|queue|connect|connect\s+to)\b/i,
  /\bqueue operation failed\b/i,
  /\bdownload client connection test failed\b/i,
  /\blibrary scan failed\b/i,
  /\binvalid\b/i,
];

const WARNING_PATTERNS: RegExp[] = [
  /\bskipped\b/i,
  /\bblocked by quality profile\b/i,
  /\bblocked by policy\b/i,
  /\bblocked by quality\b/i,
  /\bno source to queue\b/i,
  /\bno release found\b/i,
  /\bno nzb result found\b/i,
  /\bno usable imdb id\b/i,
  /\bno valid tvdb id\b/i,
  /\bno searchable title\b/i,
  /\bunknown quality profile id\b/i,
  /\bno source\b/i,
];

const SUCCESS_PATTERNS: RegExp[] = [
  /\badded\b.*\bcatalog/i,
  /\bqueued\b/i,
  /\brename apply complete\b/i,
  /\brename preview ready\b/i,
  /\bsaved\b/i,
  /\bupdated\b/i,
  /\bcreated\b/i,
  /\bdeleted\b/i,
  /\bapplied\b/i,
  /\bpassed\b/i,
  /\bcomplete\b/i,
  /\bimported\b/i,
];

export function classifyStatusToastLevel(message: string): StatusToastKind | null {
  const normalized = message.trim().toLowerCase();
  if (!normalized) {
    return null;
  }

  if (NO_TOAST_PATTERNS.some((pattern) => pattern.test(normalized))) {
    return null;
  }

  if (WARNING_PATTERNS.some((pattern) => pattern.test(normalized))) {
    return "warning";
  }

  if (ERROR_PATTERNS.some((pattern) => pattern.test(normalized))) {
    return "error";
  }

  if (SUCCESS_PATTERNS.some((pattern) => pattern.test(normalized))) {
    return "success";
  }

  return null;
}
