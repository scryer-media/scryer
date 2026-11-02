/**
 * Put text on the clipboard; resolves false only when no path takes it.
 *
 * The asynchronous clipboard exists only on a secure origin, so a Scryer
 * reached over plain HTTP at a LAN address or hostname has none, and the
 * browser also refuses it while the page is not focused. The selection
 * command still copies there, from a throwaway textarea, so the caller's
 * own field keeps its value and selection.
 */
export async function copyText(text: string): Promise<boolean> {
  const clipboard =
    typeof navigator === "undefined" ? undefined : navigator.clipboard;
  if (clipboard && typeof clipboard.writeText === "function") {
    try {
      await clipboard.writeText(text);
      return true;
    } catch {
      // Refused: fall back to the selection command below.
    }
  }
  return copyBySelection(text);
}

/**
 * Focus and select a field's whole value, so that after a refused copy the
 * user's own copy keystroke is the next thing that works.
 */
export function selectFieldById(id: string): void {
  if (typeof document === "undefined") return;
  const field = document.getElementById(id);
  if (
    field instanceof HTMLInputElement ||
    field instanceof HTMLTextAreaElement
  ) {
    field.focus({ preventScroll: true });
    field.select();
  }
}

function copyBySelection(text: string): boolean {
  if (
    typeof document === "undefined" ||
    typeof document.execCommand !== "function"
  ) {
    return false;
  }
  const previous = document.activeElement;
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.setAttribute("aria-hidden", "true");
  area.style.position = "fixed";
  area.style.top = "0";
  area.style.left = "0";
  area.style.opacity = "0";
  area.style.pointerEvents = "none";
  document.body.appendChild(area);
  let copied: boolean;
  try {
    area.focus({ preventScroll: true });
    area.select();
    area.setSelectionRange(0, text.length);
    copied = document.execCommand("copy");
  } catch {
    copied = false;
  } finally {
    area.remove();
    if (previous && typeof (previous as HTMLElement).focus === "function") {
      (previous as HTMLElement).focus({ preventScroll: true });
    }
  }
  return copied;
}
