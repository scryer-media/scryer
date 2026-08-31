export type RenameTemplateValidationIssue =
  | { kind: "empty" }
  | { kind: "unmatchedOpen" }
  | { kind: "unmatchedClose" }
  | { kind: "unknownToken"; token: string }
  | { kind: "invalidPadding"; padding: string }
  | { kind: "invalidFilter"; filter: string }
  | { kind: "invalidOptionalGroup" }
  | { kind: "nestedOptionalGroup" }
  | { kind: "unsupportedOptionalFallback" };

export type FolderTemplateValidationIssue =
  | RenameTemplateValidationIssue
  | { kind: "illegalCharacter"; character: string }
  | { kind: "missingRequiredToken"; token: string };

export type RenameTemplateSegment = {
  text: string;
  isToken: boolean;
};

export type RenameTokenFilter =
  | {
      kind: "space";
      replacement: string;
    }
  | {
      kind: "truncate";
      limit: number;
    };

export type ParsedRenameTokenSpec = {
  tokenName: string;
  lookupName: string;
  padWidth: number;
  filters: RenameTokenFilter[];
};

export const MAX_RENAME_TEMPLATE_PADDING_WIDTH = 240;

export type RenameTokenParseResult =
  | { ok: true; spec: ParsedRenameTokenSpec }
  | { ok: false; kind: "emptyToken" | "invalidFilter" | "invalidPadding"; value: string };

const SPACE_FILTER_PREFIX = "space:";
const TRUNCATE_FILTER_PREFIX = "truncate:";
const VALID_SPACE_REPLACEMENTS = new Set(["_", ".", "-", ""]);
const ILLEGAL_FOLDER_TEMPLATE_LITERAL_CHARS = new Set([
  "<", ">", ":", "\"", "/", "\\", "|", "?", "*",
]);

type ParsedOptionalRenameTemplateGroup = {
  guard: ParsedRenameTokenSpec;
  body: string;
  endIndex: number;
};

type OptionalRenameTemplateGroupParseResult =
  | { ok: true; group: ParsedOptionalRenameTemplateGroup }
  | {
    ok: false;
    kind: "unmatchedOpen" | "invalidOptionalGroup" | "nestedOptionalGroup" | "unsupportedOptionalFallback";
  };

type RenameTemplateValidationState = {
  sawRequiredToken: boolean;
};

function isIllegalFolderTemplateLiteral(character: string): boolean {
  const codePoint = character.charCodeAt(0);
  return ILLEGAL_FOLDER_TEMPLATE_LITERAL_CHARS.has(character)
    || codePoint <= 0x1f
    || (codePoint >= 0x7f && codePoint <= 0x9f);
}

function parseOptionalRenameTemplateGroup(
  template: string,
  startIndex: number,
): OptionalRenameTemplateGroupParseResult {
  let cursor = startIndex + 2;
  const guardStart = cursor;
  while (cursor < template.length && template[cursor] !== ":") {
    if (template[cursor] === "{" || template[cursor] === "}") {
      return { ok: false, kind: "invalidOptionalGroup" };
    }
    cursor++;
  }
  if (cursor === template.length) {
    return { ok: false, kind: "unmatchedOpen" };
  }

  const parsedGuard = parseRenameTemplateTokenSpec(template.slice(guardStart, cursor).trim());
  if (!parsedGuard.ok || parsedGuard.spec.padWidth !== 0 || parsedGuard.spec.filters.length > 0) {
    return { ok: false, kind: "invalidOptionalGroup" };
  }

  const bodyStart = cursor + 1;
  cursor = bodyStart;
  let escapedLiteralOpenCount = 0;
  while (cursor < template.length) {
    if (template.startsWith("{{", cursor)) {
      escapedLiteralOpenCount++;
      cursor += 2;
      continue;
    }
    if (template.startsWith("{?", cursor)) {
      return { ok: false, kind: "nestedOptionalGroup" };
    }
    if (template[cursor] === "{") {
      const closeIndex = template.indexOf("}", cursor + 1);
      if (closeIndex === -1 || template.slice(cursor + 1, closeIndex).includes("{")) {
        return { ok: false, kind: "unmatchedOpen" };
      }
      cursor = closeIndex + 1;
      continue;
    }
    if (template[cursor] === "}" && escapedLiteralOpenCount > 0) {
      cursor += template.startsWith("}}", cursor) ? 2 : 1;
      escapedLiteralOpenCount--;
      continue;
    }
    if (
      escapedLiteralOpenCount === 0
      && (template.startsWith("|else:", cursor) || template.startsWith("|?", cursor))
    ) {
      return { ok: false, kind: "unsupportedOptionalFallback" };
    }
    if (template[cursor] === "}") {
      const body = template.slice(bodyStart, cursor);
      return {
        ok: true,
        group: {
          guard: parsedGuard.spec,
          body,
          endIndex: cursor,
        },
      };
    }
    cursor++;
  }

  return { ok: false, kind: "unmatchedOpen" };
}

function validateTemplateFragment(
  template: string,
  validTokens: ReadonlySet<string>,
  state: RenameTemplateValidationState,
  requiredToken?: string,
  validateLiteral?: (character: string) => FolderTemplateValidationIssue | null,
): FolderTemplateValidationIssue | null {
  let i = 0;
  let escapedLiteralOpenCount = 0;
  while (i < template.length) {
    if (template.startsWith("{{", i)) {
      escapedLiteralOpenCount++;
      i += 2;
      continue;
    }
    if (template.startsWith("}}", i)) {
      if (escapedLiteralOpenCount > 0) {
        escapedLiteralOpenCount--;
      }
      i += 2;
      continue;
    }
    if (template.startsWith("{?", i)) {
      const parsedGroup = parseOptionalRenameTemplateGroup(template, i);
      if (!parsedGroup.ok) {
        return { kind: parsedGroup.kind };
      }
      const { guard, body, endIndex } = parsedGroup.group;
      if (!validTokens.has(guard.lookupName)) {
        return { kind: "unknownToken", token: guard.tokenName };
      }
      if (guard.lookupName === requiredToken) {
        state.sawRequiredToken = true;
      }
      const bodyIssue = validateTemplateFragment(
        body,
        validTokens,
        state,
        requiredToken,
        validateLiteral,
      );
      if (bodyIssue) {
        return bodyIssue;
      }
      i = endIndex + 1;
      continue;
    }
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1 || template.slice(i + 1, closeIndex).includes("{")) {
        return { kind: "unmatchedOpen" };
      }
      const parsed = parseRenameTemplateTokenSpec(template.slice(i + 1, closeIndex));
      if (!parsed.ok) {
        if (parsed.kind === "invalidFilter") {
          return { kind: "invalidFilter", filter: parsed.value };
        }
        if (parsed.kind === "invalidPadding") {
          return { kind: "invalidPadding", padding: parsed.value };
        }
        return { kind: "unknownToken", token: parsed.value };
      }
      if (!validTokens.has(parsed.spec.lookupName)) {
        return { kind: "unknownToken", token: parsed.spec.tokenName };
      }
      if (parsed.spec.lookupName === requiredToken) {
        state.sawRequiredToken = true;
      }
      i = closeIndex + 1;
      continue;
    }
    if (template[i] === "}") {
      if (escapedLiteralOpenCount > 0) {
        escapedLiteralOpenCount--;
        i++;
        continue;
      }
      return { kind: "unmatchedClose" };
    }
    const literalIssue = validateLiteral?.(template[i]);
    if (literalIssue) {
      return literalIssue;
    }
    i++;
  }

  return null;
}

export function validateFolderTemplateSyntax(
  template: string,
  validTokens: ReadonlySet<string>,
  requiredToken?: string,
): FolderTemplateValidationIssue | null {
  const trimmed = template.trim();
  if (!trimmed) {
    return { kind: "empty" };
  }

  const state = { sawRequiredToken: requiredToken === undefined };
  const issue = validateTemplateFragment(
    trimmed,
    validTokens,
    state,
    requiredToken,
    (character) => isIllegalFolderTemplateLiteral(character)
      ? { kind: "illegalCharacter", character }
      : null,
  );
  if (issue) {
    return issue;
  }

  return state.sawRequiredToken
    ? null
    : { kind: "missingRequiredToken", token: requiredToken ?? "" };
}

export function validateRenameTemplateSyntax(
  template: string,
  validTokens: ReadonlySet<string>,
): RenameTemplateValidationIssue | null {
  if (!template.trim()) {
    return { kind: "empty" };
  }

  return validateTemplateFragment(template, validTokens, { sawRequiredToken: true }) as RenameTemplateValidationIssue | null;
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
    if (template.startsWith("{?", i)) {
      const parsedGroup = parseOptionalRenameTemplateGroup(template, i);
      if (!parsedGroup.ok) return null;
      const { guard, body, endIndex } = parsedGroup.group;
      if (!validTokens.has(guard.lookupName)) return null;
      if ((sampleValues[guard.lookupName] ?? "").trim()) {
        const renderedBody = applyRenameTemplatePreview(body, validTokens, sampleValues);
        if (renderedBody === null) return null;
        result += renderedBody;
      }
      i = endIndex + 1;
      continue;
    }
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) return null;
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) return null;
      const parsed = parseRenameTemplateTokenSpec(inner);
      if (!parsed.ok || !validTokens.has(parsed.spec.lookupName)) return null;
      let value = sampleValues[parsed.spec.lookupName] ?? "";
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
      if (template.startsWith("{?", cursor)) {
        const parsedGroup = parseOptionalRenameTemplateGroup(template, cursor);
        if (
          parsedGroup.ok
          && validTokens.has(parsedGroup.group.guard.lookupName)
          && validateRenameTemplateSyntax(template.slice(cursor, parsedGroup.group.endIndex + 1), validTokens) === null
        ) {
          pushPlain();
          segments.push({
            text: template.slice(cursor, parsedGroup.group.endIndex + 1),
            isToken: true,
          });
          cursor = parsedGroup.group.endIndex + 1;
          continue;
        }
      }
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

export function parseRenameTemplateTokenSpec(inner: string): RenameTokenParseResult {
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

  let padWidth = 0;
  if (colonIdx >= 0) {
    const rawPadding = tokenCore.slice(colonIdx + 1).trim();
    if (!/^\d+$/.test(rawPadding)) {
      return { ok: false, kind: "invalidPadding", value: rawPadding };
    }
    padWidth = Number(rawPadding);
    if (!Number.isSafeInteger(padWidth) || padWidth > MAX_RENAME_TEMPLATE_PADDING_WIDTH) {
      return { ok: false, kind: "invalidPadding", value: rawPadding };
    }
  }
  const filters: RenameTokenFilter[] = [];

  for (const rawFilter of parts) {
    const filter = rawFilter.trim();
    if (filter.startsWith(SPACE_FILTER_PREFIX)) {
      const replacement = filter.slice(SPACE_FILTER_PREFIX.length);
      if (!VALID_SPACE_REPLACEMENTS.has(replacement)) {
        return { ok: false, kind: "invalidFilter", value: filter };
      }
      filters.push({ kind: "space", replacement });
      continue;
    }

    if (filter.startsWith(TRUNCATE_FILTER_PREFIX)) {
      const rawLimit = filter.slice(TRUNCATE_FILTER_PREFIX.length);
      if (!/^\d+$/.test(rawLimit)) {
        return { ok: false, kind: "invalidFilter", value: filter };
      }
      const limit = Number.parseInt(rawLimit, 10);
      if (limit <= 0) {
        return { ok: false, kind: "invalidFilter", value: filter };
      }
      filters.push({ kind: "truncate", limit });
      continue;
    }

    return { ok: false, kind: "invalidFilter", value: filter };
  }

  return {
    ok: true,
    spec: {
      tokenName,
      lookupName: tokenName.toLowerCase(),
      padWidth,
      filters,
    },
  };
}

export function applyRenameTokenFilters(value: string, filters: RenameTokenFilter[]): string {
  return filters.reduce((current, filter) => {
    switch (filter.kind) {
      case "space":
        return current.replace(/\s/g, filter.replacement);
      case "truncate":
        return Array.from(current).slice(0, filter.limit).join("");
      default:
        return current;
    }
  }, value);
}
