export type RenameTemplateValidationIssue =
  | { kind: "empty" }
  | { kind: "unmatchedOpen" }
  | { kind: "unmatchedClose" }
  | { kind: "unknownToken"; token: string }
  | { kind: "invalidFilter"; filter: string };

export type RenameTemplateSegment = {
  text: string;
  isToken: boolean;
};

type RenameTokenFilter = {
  kind: "space";
  replacement: string;
};

type ParsedRenameTokenSpec = {
  tokenName: string;
  lookupName: string;
  padWidth: number;
  filters: RenameTokenFilter[];
};

type RenameTokenParseResult =
  | { ok: true; spec: ParsedRenameTokenSpec }
  | { ok: false; kind: "emptyToken" | "invalidFilter"; value: string };

const SPACE_FILTER_PREFIX = "space:";
const VALID_SPACE_REPLACEMENTS = new Set(["_", ".", "-", ""]);

// Sonarr/Radarr-style multi-word aliases, e.g. {Series.Title}, {Episode.Title}.
// Mirrors TOKEN_NAME_ALIASES in crates/scryer-application/src/library/rename.rs.
const TOKEN_NAME_ALIASES: ReadonlyArray<readonly [string[], string]> = [
  [["movie", "title"], "title"],
  [["series", "title"], "title"],
  [["episode", "title"], "episode_title"],
  [["release", "year"], "year"],
  [["quality", "full"], "quality"],
];

type ResolvedTokenName = {
  canonical: string;
  impliedSeparator: string | null;
};

// Gated on an uppercase letter so existing lowercase tokens (episode_title,
// tmdb_id, ...) never enter alias resolution even though they contain "_".
function resolveTemplateTokenName(rawName: string): ResolvedTokenName {
  const trimmed = rawName.trim();
  if (!/[A-Z]/.test(trimmed)) {
    return { canonical: trimmed.toLowerCase(), impliedSeparator: null };
  }

  const words: string[] = [];
  let current = "";
  let separator: string | null = null;
  for (const ch of trimmed) {
    if (ch === "." || ch === "_" || /\s/.test(ch)) {
      if (current) {
        words.push(current.toLowerCase());
        current = "";
      }
      if (separator === null) {
        separator = /\s/.test(ch) ? " " : ch;
      }
    } else {
      current += ch;
    }
  }
  if (current) {
    words.push(current.toLowerCase());
  }

  if (words.length < 2) {
    return { canonical: trimmed.toLowerCase(), impliedSeparator: null };
  }

  const lookup = words.join(" ");
  const match = TOKEN_NAME_ALIASES.find(([alias]) => alias.join(" ") === lookup);
  if (!match) {
    return { canonical: trimmed.toLowerCase(), impliedSeparator: null };
  }

  return {
    canonical: match[1],
    impliedSeparator: separator === " " || separator === null ? null : separator,
  };
}

function parseRenameTemplatePadWidth(fmt: string): number {
  const trimmed = fmt.trim();
  if (trimmed.length > 0 && /^0+$/.test(trimmed)) {
    // Sonarr-style {season:00} pads to the width of the zero run.
    return trimmed.length;
  }
  const parsed = Number.parseInt(trimmed, 10);
  return Number.isNaN(parsed) ? 0 : parsed;
}

export function validateRenameTemplateSyntax(
  template: string,
  validTokens: ReadonlySet<string>,
): RenameTemplateValidationIssue | null {
  if (!template.trim()) {
    return { kind: "empty" };
  }

  let i = 0;
  let escapedLiteralOpenCount = 0;
  while (i < template.length) {
    if (template.startsWith("{{", i)) {
      escapedLiteralOpenCount += 1;
      i += 2;
      continue;
    }
    if (template.startsWith("}}", i)) {
      if (escapedLiteralOpenCount > 0) {
        escapedLiteralOpenCount -= 1;
      }
      i += 2;
      continue;
    }
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) {
        return { kind: "unmatchedOpen" };
      }
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) {
        return { kind: "unmatchedOpen" };
      }
      const parsed = parseRenameTemplateTokenSpec(inner);
      if (!parsed.ok) {
        return parsed.kind === "invalidFilter"
          ? { kind: "invalidFilter", filter: parsed.value }
          : { kind: "unknownToken", token: parsed.value };
      }
      if (!validTokens.has(parsed.spec.lookupName)) {
        return { kind: "unknownToken", token: parsed.spec.tokenName };
      }
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      if (escapedLiteralOpenCount > 0) {
        escapedLiteralOpenCount -= 1;
        i++;
        continue;
      }
      return { kind: "unmatchedClose" };
    } else {
      i++;
    }
  }

  return null;
}

export function applyRenameTemplatePreview(
  template: string,
  validTokens: ReadonlySet<string>,
  sampleValues: Record<string, string>,
): string | null {
  if (!template.trim()) {
    return null;
  }

  let result = "";
  let i = 0;
  let escapedLiteralOpenCount = 0;
  while (i < template.length) {
    if (template.startsWith("{{", i)) {
      result += "{";
      escapedLiteralOpenCount += 1;
      i += 2;
      continue;
    }
    if (template.startsWith("}}", i)) {
      result += "}";
      if (escapedLiteralOpenCount > 0) {
        escapedLiteralOpenCount -= 1;
      }
      i += 2;
      continue;
    }
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) return null;
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) return null;
      const parsed = parseRenameTemplateTokenSpec(inner);
      if (!parsed.ok || !validTokens.has(parsed.spec.lookupName)) return null;
      let value = sampleValues[parsed.spec.lookupName] ?? parsed.spec.tokenName;
      if (parsed.spec.padWidth > 0 && /^\d+$/.test(value)) {
        value = value.padStart(parsed.spec.padWidth, "0");
      }
      result += applyRenameTokenFilters(value, parsed.spec.filters);
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      if (escapedLiteralOpenCount > 0) {
        result += "}";
        escapedLiteralOpenCount -= 1;
        i++;
        continue;
      }
      return null;
    } else {
      result += template[i];
      i++;
    }
  }

  return result;
}

export function splitRenameTemplateSegments(
  template: string,
  validTokens: ReadonlySet<string>,
): RenameTemplateSegment[] {
  if (!template) {
    return [];
  }

  const segments: RenameTemplateSegment[] = [];
  let plain = "";
  let cursor = 0;

  const pushPlain = () => {
    if (plain.length === 0) {
      return;
    }
    segments.push({ text: plain, isToken: false });
    plain = "";
  };

  while (cursor < template.length) {
    if (template.startsWith("{{", cursor) || template.startsWith("}}", cursor)) {
      plain += template.slice(cursor, cursor + 2);
      cursor += 2;
      continue;
    }

    if (template[cursor] === "{") {
      const closeIndex = template.indexOf("}", cursor + 1);
      if (closeIndex !== -1) {
        const inner = template.slice(cursor + 1, closeIndex);
        const parsed = inner.includes("{")
          ? null
          : parseRenameTemplateTokenSpec(inner);
        if (parsed?.ok && validTokens.has(parsed.spec.lookupName)) {
          pushPlain();
          segments.push({
            text: template.slice(cursor, closeIndex + 1),
            isToken: true,
          });
          cursor = closeIndex + 1;
          continue;
        }
      }
    }

    plain += template[cursor];
    cursor++;
  }

  pushPlain();
  return segments;
}

function parseRenameTemplateTokenSpec(inner: string): RenameTokenParseResult {
  const parts = inner.split("|");
  const tokenCore = parts.shift()?.trim() ?? "";
  if (!tokenCore) {
    return { ok: false, kind: "emptyToken", value: inner };
  }

  const colonIdx = tokenCore.indexOf(":");
  const tokenName = (colonIdx >= 0 ? tokenCore.slice(0, colonIdx) : tokenCore).trim();
  if (!tokenName) {
    return { ok: false, kind: "emptyToken", value: inner };
  }

  const padWidth =
    colonIdx >= 0 ? parseRenameTemplatePadWidth(tokenCore.slice(colonIdx + 1)) : 0;
  const filters: RenameTokenFilter[] = [];

  for (const rawFilter of parts) {
    const filter = rawFilter.trim();
    if (!filter.startsWith(SPACE_FILTER_PREFIX)) {
      return { ok: false, kind: "invalidFilter", value: filter };
    }
    const replacement = filter.slice(SPACE_FILTER_PREFIX.length);
    if (!VALID_SPACE_REPLACEMENTS.has(replacement)) {
      return { ok: false, kind: "invalidFilter", value: filter };
    }
    filters.push({ kind: "space", replacement });
  }

  const resolved = resolveTemplateTokenName(tokenName);
  if (filters.length === 0 && resolved.impliedSeparator !== null) {
    filters.push({ kind: "space", replacement: resolved.impliedSeparator });
  }

  return {
    ok: true,
    spec: {
      tokenName,
      lookupName: resolved.canonical,
      padWidth,
      filters,
    },
  };
}

function applyRenameTokenFilters(value: string, filters: RenameTokenFilter[]): string {
  return filters.reduce((current, filter) => {
    switch (filter.kind) {
      case "space":
        return current.replace(/\s/g, filter.replacement);
      default:
        return current;
    }
  }, value);
}
