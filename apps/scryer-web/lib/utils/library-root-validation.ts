import type { LocalPathStyle } from "./local-path-style.ts";

type GraphQlErrorLike = {
  extensions?: Record<string, unknown> | null;
};

type CombinedErrorLike = {
  graphQLErrors?: readonly GraphQlErrorLike[];
};

export type LibraryRootValidationResult = {
  validPaths: string[];
  invalidPaths: string[];
  unavailable: boolean;
};

function isWindowsDriveRoot(path: string): boolean {
  return /^[a-z]:[\\/]$/iu.test(path);
}

function isUncShareRoot(path: string): boolean {
  if (!/^[\\/]{2}[^\\/]/u.test(path)) return false;
  return (
    path
      .slice(2)
      .split(/[\\/]+/u)
      .filter(Boolean).length === 2
  );
}

export function trimLibraryRootPath(path: string): string {
  const trimmed = path.trim();
  if (!trimmed) return "";
  if (/^[\\/]+$/u.test(trimmed)) return trimmed[0] ?? "/";
  if (isWindowsDriveRoot(trimmed) || isUncShareRoot(trimmed)) return trimmed;
  return trimmed.replace(/[\\/]+$/u, "");
}

export function normalizeComparableLibraryRootPath(
  path: string,
  pathStyle?: LocalPathStyle,
): string {
  const storedPath = trimLibraryRootPath(path);
  const canonicalPath = isUncShareRoot(storedPath)
    ? storedPath.replace(/[\\/]+$/u, "")
    : storedPath;
  const normalizedPath = canonicalPath.replaceAll("\\", "/");
  return pathStyle === "windows"
    ? normalizedPath.toLowerCase()
    : normalizedPath;
}

type LibraryRootDraftLike = {
  path: string;
  isDefault: boolean;
};

type LibraryRootCollectionLike = {
  id: string;
  name: string;
  roots: readonly { path: string }[];
};

export function normalizeLibraryRootDrafts<T extends LibraryRootDraftLike>(
  roots: readonly T[],
  pathStyle?: LocalPathStyle,
): T[] {
  const seen = new Set<string>();
  let hasDefault = false;
  const normalized: T[] = [];

  roots.forEach((root) => {
    const path = trimLibraryRootPath(root.path);
    const comparablePath = normalizeComparableLibraryRootPath(path, pathStyle);
    if (!path || seen.has(comparablePath)) {
      return;
    }
    seen.add(comparablePath);
    const isDefault = root.isDefault && !hasDefault;
    hasDefault ||= isDefault;
    normalized.push({ ...root, path, isDefault });
  });

  if (normalized.length > 0 && !hasDefault) {
    normalized[0] = { ...normalized[0], isDefault: true };
  }

  return normalized;
}

export function findConflictingLibraryNamesByRootPath(
  draftRoots: readonly { path: string }[],
  libraries: readonly LibraryRootCollectionLike[],
  currentLibraryId: string | null,
  pathStyle?: LocalPathStyle,
): Map<string, string[]> {
  const otherLibrariesByRootPath = new Map<string, string[]>();

  libraries.forEach((library) => {
    if (library.id === currentLibraryId) {
      return;
    }
    library.roots.forEach((root) => {
      const comparablePath = normalizeComparableLibraryRootPath(
        root.path,
        pathStyle,
      );
      if (!comparablePath) {
        return;
      }
      const names = otherLibrariesByRootPath.get(comparablePath) ?? [];
      if (!names.includes(library.name)) {
        names.push(library.name);
      }
      otherLibrariesByRootPath.set(comparablePath, names);
    });
  });

  const conflicts = new Map<string, string[]>();
  draftRoots.forEach((root) => {
    const comparablePath = normalizeComparableLibraryRootPath(
      root.path,
      pathStyle,
    );
    const names = otherLibrariesByRootPath.get(comparablePath);
    if (names?.length) {
      conflicts.set(root.path, names);
    }
  });
  return conflicts;
}

export function isExplicitPathValidationError(error: unknown): boolean {
  if (typeof error !== "object" || error === null) return false;
  const graphQLErrors = (error as CombinedErrorLike).graphQLErrors ?? [];
  return graphQLErrors.some(
    (graphQlError) => graphQlError.extensions?.code === "VALIDATION_ERROR",
  );
}

export async function validateLibraryRootPaths(
  paths: string[],
  validate: (path: string) => Promise<unknown | null | undefined>,
  concurrency = 4,
): Promise<LibraryRootValidationResult> {
  const validPaths: string[] = [];
  const invalidPaths: string[] = [];
  let unavailable = false;
  let nextIndex = 0;

  const worker = async () => {
    while (nextIndex < paths.length) {
      const index = nextIndex;
      nextIndex += 1;
      const path = paths[index];
      if (path === undefined) continue;

      try {
        const error = await validate(path);
        if (!error) {
          validPaths.push(path);
          continue;
        }
        if (isExplicitPathValidationError(error)) {
          invalidPaths.push(path);
        } else {
          unavailable = true;
        }
      } catch {
        unavailable = true;
      }
    }
  };

  const workerCount = Math.min(Math.max(1, concurrency), paths.length);
  await Promise.all(Array.from({ length: workerCount }, () => worker()));
  return { validPaths, invalidPaths, unavailable };
}
