// Saving a `fetch` response to the operator's disk. Shared by the backup
// download and by the Indexers > Search browser download (D17): both answer
// with `Content-Disposition: attachment` and neither can use a plain link,
// because the request carries an Authorization header.

type SaveFilePickerWindow = Window & {
  showSaveFilePicker?: (options: {
    suggestedName: string;
  }) => Promise<{
    createWritable: () => Promise<WritableStream<Uint8Array>>;
  }>;
};

/**
 * The filename an attachment response names, preferring RFC 6266's `filename*`
 * (the server sends the real, possibly non-ASCII, release name there).
 */
export function filenameFromContentDisposition(
  header: string | null,
  fallback: string,
): string {
  if (!header) {
    return fallback;
  }
  const extended = /filename\*\s*=\s*UTF-8''([^;]+)/i.exec(header);
  if (extended) {
    try {
      const decoded = decodeURIComponent(extended[1].trim()).trim();
      if (decoded) {
        return decoded;
      }
    } catch {
      // Fall through to the quoted form below.
    }
  }
  const quoted = /filename\s*=\s*"((?:[^"\\]|\\.)*)"/i.exec(header);
  if (quoted) {
    const unescaped = quoted[1].replaceAll(/\\(.)/g, "$1").trim();
    if (unescaped) {
      return unescaped;
    }
  }
  const bare = /filename\s*=\s*([^;]+)/i.exec(header);
  const trimmed = bare?.[1].trim().replaceAll(/^"|"$/g, "").trim();
  return trimmed || fallback;
}

/** The message a non-2xx download response carries, JSON body or plain text. */
export async function readResponseErrorMessage(
  response: Response,
  fallback: string,
): Promise<string> {
  const contentType = response.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    const payload = await response.json().catch(() => null) as {
      error?: string;
      error_id?: string;
    } | null;
    const message = payload?.error?.trim();
    if (message) {
      const errorId = payload?.error_id?.trim();
      return errorId ? `${message}. Reference ID: ${errorId}` : message;
    }
  }

  const text = await response.text().catch(() => "");
  return text.trim() || fallback;
}

/**
 * Stream the response to a file the operator picks, falling back to an
 * object-URL anchor where the File System Access API is unavailable.
 */
export async function saveDownloadResponse(response: Response, filename: string): Promise<void> {
  const windowWithPicker = window as SaveFilePickerWindow;
  if (response.body && typeof windowWithPicker.showSaveFilePicker === "function") {
    try {
      const handle = await windowWithPicker.showSaveFilePicker({
        suggestedName: filename,
      });
      const writable = await handle.createWritable();
      await response.body.pipeTo(writable);
      return;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      throw error;
    }
  }

  const blob = await response.blob();
  const downloadUrl = window.URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = downloadUrl;
  link.download = filename;
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => {
    window.URL.revokeObjectURL(downloadUrl);
  }, 0);
}
