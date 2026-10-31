export type LocalPathStyle = "unix" | "windows";
export type RuntimePathStyleValue = "UNIX" | "WINDOWS";

function isWindowsDriveAbsolutePath(path: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(path);
}

function isWindowsUncAbsolutePath(path: string): boolean {
  return /^\\\\[^\\/]+[\\/][^\\/]+/.test(path);
}

export function inferLocalPathStyleFromPath(
  path: string | null | undefined,
): LocalPathStyle | null {
  const trimmed = path?.trim() ?? "";
  if (!trimmed) {
    return null;
  }

  if (
    isWindowsDriveAbsolutePath(trimmed) ||
    isWindowsUncAbsolutePath(trimmed)
  ) {
    return "windows";
  }

  if (trimmed.startsWith("/")) {
    return "unix";
  }

  return null;
}

export function localPathStyleFromRuntimeValue(
  value: string | null | undefined,
): LocalPathStyle | undefined {
  if (value === "WINDOWS") {
    return "windows";
  }
  if (value === "UNIX") {
    return "unix";
  }
  return undefined;
}

export function isAbsoluteLocalPathForStyle(
  path: string,
  style: LocalPathStyle,
): boolean {
  if (style === "windows") {
    return isWindowsDriveAbsolutePath(path) || isWindowsUncAbsolutePath(path);
  }

  return path.startsWith("/");
}

/**
 * Format-validate a local path against the server's path style, tolerating an
 * unknown style. While the runtime style is still unknown (e.g. the
 * `runtimeInfo` query has not resolved yet, or an older server does not report
 * it), the format check is skipped so valid paths are not transiently flagged
 * as invalid. Server-side validation remains the source of truth on save.
 */
export function isLocalPathFormatValidForStyle(
  path: string,
  style: LocalPathStyle | undefined,
): boolean {
  if (!style) {
    return true;
  }
  return isAbsoluteLocalPathForStyle(path, style);
}
