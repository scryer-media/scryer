import type { MediaServerConnectionDraft } from "../types/index.ts";
import { normalizeUrlInput } from "./url-input.ts";

/**
 * The media server draft as it will be stored, with both address boxes taken
 * as they were typed.
 *
 * An operator copies the server address out of Plex, Jellyfin or Emby's own
 * settings page, or types the LAN address they always use: `192.168.1.5:32400`
 * or `jellyfin.lan`. Both mean what they plainly mean, so a missing scheme is
 * chosen from the address rather than refused, and a trailing slash is
 * dropped. A scheme the operator wrote is never rewritten. The public URL is
 * handled the same way, since it is pasted out of the same places.
 *
 * Nothing to do means the same object back, so the form is only rewritten
 * when something actually changed.
 */
export function normalizeMediaServerConnectionDraft(
  draft: MediaServerConnectionDraft,
): MediaServerConnectionDraft {
  const baseUrl = normalizeAddress(draft.baseUrl);
  const externalUrl = normalizeAddress(draft.externalUrl);
  if (baseUrl === draft.baseUrl && externalUrl === draft.externalUrl) {
    return draft;
  }
  return { ...draft, baseUrl, externalUrl };
}

/** An empty box stays empty; Plex may have no local address at all. */
function normalizeAddress(value: string): string {
  return value.trim() === "" ? value : normalizeUrlInput(value);
}
