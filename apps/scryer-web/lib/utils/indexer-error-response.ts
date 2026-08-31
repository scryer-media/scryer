export const MAX_INDEXER_ERROR_BODY_PREVIEW_BYTES = 1024 * 1024;

export type IndexerErrorBodyFormat = "json" | "xml" | "text" | "binary";

export type IndexerErrorBodyPresentation = {
  byteLength: number;
  format: IndexerErrorBodyFormat;
  formattedText: string | null;
  rawText: string | null;
  truncated: boolean;
};

function normalizedBase64(value: string): string {
  return value.replace(/\s/g, "");
}

export function decodedBase64Length(value: string): number {
  const normalized = normalizedBase64(value);
  if (normalized.length === 0) return 0;
  const padding = normalized.endsWith("==") ? 2 : normalized.endsWith("=") ? 1 : 0;
  return Math.max(0, Math.floor((normalized.length * 3) / 4) - padding);
}

function base64ToBytes(value: string, maximumBytes?: number): Uint8Array {
  const normalized = normalizedBase64(value);
  const encoded = maximumBytes == null
    ? normalized
    : normalized.slice(0, Math.ceil(maximumBytes / 3) * 4);
  const binary = atob(encoded);
  const length = maximumBytes == null ? binary.length : Math.min(binary.length, maximumBytes);
  const bytes = new Uint8Array(length);
  for (let index = 0; index < length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

export function decodeIndexerErrorBody(value: string): Uint8Array {
  return base64ToBytes(value);
}

function decodeUtf8(bytes: Uint8Array, allowIncompleteTrailingCodePoint: boolean): string | null {
  const maximumTrim = allowIncompleteTrailingCodePoint ? Math.min(3, bytes.length) : 0;
  for (let trim = 0; trim <= maximumTrim; trim += 1) {
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(
        trim === 0 ? bytes : bytes.subarray(0, bytes.length - trim),
      );
    } catch {
      // A capped preview may end partway through one UTF-8 code point.
    }
  }
  return null;
}

function declaredFormat(contentType: string | null | undefined): IndexerErrorBodyFormat | null {
  const normalized = contentType?.split(";", 1)[0]?.trim().toLowerCase() ?? "";
  if (normalized === "application/json" || normalized.endsWith("+json")) return "json";
  if (normalized === "application/xml" || normalized === "text/xml" || normalized.endsWith("+xml")) return "xml";
  if (normalized.startsWith("text/")) return "text";
  return null;
}

function sniffFormat(text: string): IndexerErrorBodyFormat {
  const trimmed = text.trimStart();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) return "json";
  if (trimmed.startsWith("<")) return "xml";
  return "text";
}

function prettyPrintXml(value: string): string | null {
  const tokens = value.match(
    /<\?[^]*?\?>|<!--[^]*?-->|<!\[CDATA\[[^]*?\]\]>|<![^>]*>|<[^>]+>|[^<]+/g,
  );
  if (!tokens || tokens.join("") !== value) return null;
  if (/\bxml:space\s*=\s*(["'])preserve\1/.test(value)) return null;

  const lines: string[] = [];
  const stack: string[] = [];
  let rootSeen = false;
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index] ?? "";
    const trimmed = token.trim();
    if (!trimmed) continue;
    const closing = trimmed.match(/^<\/\s*([^\s>]+)\s*>$/);
    const opening = trimmed.match(/^<\s*([^!?/\s>]+)(?:\s[^>]*)?>$/);
    const selfClosing = /\/\s*>$/.test(trimmed);
    const special = /^<(?:\?|!)/.test(trimmed);

    if (closing) {
      if (stack.pop() !== closing[1]) return null;
      lines.push(`${"  ".repeat(stack.length)}${trimmed}`);
      continue;
    }
    if (opening) {
      if (stack.length === 0) {
        if (rootSeen) return null;
        rootSeen = true;
      }

      const next = tokens[index + 1];
      const following = tokens[index + 2];
      const matchingClose = following?.match(/^<\/\s*([^\s>]+)\s*>$/);
      const nextIsText = next != null && (
        !next.startsWith("<") || next.startsWith("<![CDATA[")
      );
      if (!selfClosing && nextIsText && matchingClose?.[1] === opening[1]) {
        lines.push(`${"  ".repeat(stack.length)}${trimmed}${next}${following}`);
        index += 2;
        continue;
      }

      lines.push(`${"  ".repeat(stack.length)}${trimmed}`);
      if (!selfClosing) stack.push(opening[1]);
      continue;
    }
    if (special) {
      if (/^<\?xml\b/i.test(trimmed) && rootSeen) return null;
      if (/^<!DOCTYPE\b/i.test(trimmed) && (rootSeen || stack.length > 0)) return null;
      lines.push(`${"  ".repeat(stack.length)}${trimmed}`);
      continue;
    }

    // Significant text outside a text-only leaf is mixed content. Formatting
    // it would change the captured response, so leave that response raw.
    return null;
  }
  return rootSeen && stack.length === 0 ? lines.join("\n") : null;
}

export function presentIndexerErrorBody(
  bodyBase64: string,
  contentType?: string | null,
): IndexerErrorBodyPresentation {
  const byteLength = decodedBase64Length(bodyBase64);
  const truncated = byteLength > MAX_INDEXER_ERROR_BODY_PREVIEW_BYTES;
  const previewBytes = base64ToBytes(
    bodyBase64,
    truncated ? MAX_INDEXER_ERROR_BODY_PREVIEW_BYTES : undefined,
  );
  const text = decodeUtf8(previewBytes, truncated);
  if (text == null) {
    return { byteLength, format: "binary", formattedText: null, rawText: null, truncated };
  }

  const format = declaredFormat(contentType) ?? sniffFormat(text);
  let formattedText: string | null = null;
  if (!truncated && format === "json") {
    try {
      formattedText = JSON.stringify(JSON.parse(text), null, 2);
    } catch {
      formattedText = null;
    }
  } else if (!truncated && format === "xml") {
    formattedText = prettyPrintXml(text);
  }

  return { byteLength, format, formattedText, rawText: text, truncated };
}

export function isSensitiveIndexerErrorHeader(name: string): boolean {
  const normalized = name.trim().toLowerCase();
  return normalized === "authorization" ||
    normalized === "proxy-authorization" ||
    normalized === "cookie" ||
    normalized === "set-cookie" ||
    normalized.includes("api-key") ||
    normalized.includes("apikey") ||
    normalized.includes("api_key") ||
    normalized.includes("token") ||
    normalized.includes("secret");
}

export function indexerErrorDownloadExtension(
  format: IndexerErrorBodyFormat,
): string {
  if (format === "json") return "json";
  if (format === "xml") return "xml";
  if (format === "text") return "txt";
  return "bin";
}
