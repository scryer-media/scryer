import {
  STANDALONE_VIDEO_EXTENSIONS,
  VIDEO_EXTENSIONS,
} from "./media-formats.generated.ts";

function normalizedFileName(path: string): string {
  return (path.split(/[\\/]/).pop() ?? "")
    .replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "")
    .replace(/[\t\n\v\f\r "'`_.]+$/, "");
}

export function canonicalVideoExtension(path: string): string | undefined {
  const name = normalizedFileName(path);
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return undefined;
  const extension = name
    .slice(dot + 1)
    .replace(/[A-Z]/g, (letter) => letter.toLowerCase());
  return VIDEO_EXTENSIONS.find((known) => known === extension);
}

/** Only a standalone library suffix is a file extension worth stripping. */
export function videoFileStem(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const normalized = normalizedFileName(path);
  const extension = canonicalVideoExtension(path);
  return extension &&
    (STANDALONE_VIDEO_EXTENSIONS as readonly string[]).includes(extension)
    ? normalized.slice(0, normalized.lastIndexOf("."))
    : name;
}
