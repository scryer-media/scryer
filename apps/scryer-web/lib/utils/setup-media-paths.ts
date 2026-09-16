import type { LibraryRecord } from "../types/titles.ts";
import {
  normalizeLibraryRootDrafts,
  validateLibraryRootPaths,
} from "./library-root-validation.ts";

export type SetupMediaPathField = "movies" | "series" | "anime";

export const SETUP_MEDIA_PATH_FIELDS: readonly SetupMediaPathField[] = [
  "movies",
  "series",
  "anime",
];

const FIELD_FACETS: Record<SetupMediaPathField, string> = {
  movies: "MOVIE",
  series: "SERIES",
  anime: "ANIME",
};

export const SETUP_MEDIA_PATH_LABEL_KEYS: Record<SetupMediaPathField, string> = {
  movies: "setup.moviesPath",
  series: "setup.seriesPath",
  anime: "setup.animePath",
};

export type SetupRoot = { path: string; isDefault: boolean };
export type SetupMediaRoots = Record<SetupMediaPathField, SetupRoot[]>;

/** Each facet's default library: where setup writes that facet's roots. */
export type SetupMediaLibraries = Partial<
  Record<SetupMediaPathField, { id: string; roots: SetupRoot[] }>
>;

export type SetupMediaPathValidationState = {
  invalidPaths: string[];
  unavailable: boolean;
};

/** What the step shows before the libraries have been read. */
export function bootstrapSetupMediaRoots(): SetupMediaRoots {
  return {
    movies: [{ path: "/data/movies", isDefault: true }],
    series: [{ path: "/data/series", isDefault: true }],
    anime: [{ path: "/data/anime", isDefault: true }],
  };
}

export function setupMediaLibraries(
  libraries: readonly Pick<LibraryRecord, "id" | "facet" | "isDefault" | "roots">[],
): SetupMediaLibraries {
  const result: SetupMediaLibraries = {};
  for (const field of SETUP_MEDIA_PATH_FIELDS) {
    const library = libraries.find(
      (entry) => entry.isDefault && entry.facet === FIELD_FACETS[field],
    );
    if (library) {
      result[field] = {
        id: library.id,
        roots: normalizeLibraryRootDrafts(
          library.roots.map(({ path, isDefault }) => ({ path, isDefault })),
        ),
      };
    }
  }
  return result;
}

/** The roots each facet shows: its library's, or the bootstrap default. */
export function setupMediaRootsFromLibraries(
  libraries: SetupMediaLibraries,
): SetupMediaRoots {
  const roots = bootstrapSetupMediaRoots();
  for (const field of SETUP_MEDIA_PATH_FIELDS) {
    const library = libraries[field];
    if (library && library.roots.length > 0) roots[field] = library.roots;
  }
  return roots;
}

export function addSetupRoot(roots: SetupRoot[], path: string): SetupRoot[] {
  return normalizeLibraryRootDrafts([
    ...roots,
    { path, isDefault: roots.length === 0 },
  ]);
}

export function replaceSetupRoot(
  roots: SetupRoot[],
  index: number,
  path: string,
): SetupRoot[] {
  // A path already listed elsewhere would collapse two rows into one.
  if (roots.some((root, i) => i !== index && root.path === path.trim())) {
    return roots;
  }
  return normalizeLibraryRootDrafts(
    roots.map((root, i) => (i === index ? { ...root, path } : root)),
  );
}

export function removeSetupRoot(roots: SetupRoot[], index: number): SetupRoot[] {
  return normalizeLibraryRootDrafts(roots.filter((_, i) => i !== index));
}

export function setDefaultSetupRoot(roots: SetupRoot[], index: number): SetupRoot[] {
  return roots.map((root, i) => ({ ...root, isDefault: i === index }));
}

function sameRoots(left: SetupRoot[], right: SetupRoot[]) {
  const a = normalizeLibraryRootDrafts(left);
  const b = normalizeLibraryRootDrafts(right);
  return (
    a.length === b.length &&
    a.every(
      (root, i) => root.path === b[i].path && root.isDefault === b[i].isDefault,
    )
  );
}

export type SetupRootSave = {
  field: SetupMediaPathField;
  libraryId: string;
  roots: SetupRoot[];
};

/**
 * The libraries setup should rewrite: those whose roots were changed. A facet
 * left with no roots is left alone — a library always keeps at least one — so
 * re-running setup never rewrites roots it did not touch.
 */
export function plannedSetupRootSaves(
  roots: SetupMediaRoots,
  libraries: SetupMediaLibraries,
): SetupRootSave[] {
  return SETUP_MEDIA_PATH_FIELDS.flatMap((field) => {
    const library = libraries[field];
    const next = normalizeLibraryRootDrafts(roots[field]);
    if (!library || next.length === 0 || sameRoots(next, library.roots)) {
      return [];
    }
    return [{ field, libraryId: library.id, roots: next }];
  });
}

type AdvisorySetupMediaPathSaveOptions = {
  roots: SetupMediaRoots;
  validatePath: (path: string) => Promise<unknown | null | undefined>;
  save: () => Promise<void>;
  onValidation: (state: SetupMediaPathValidationState) => void;
  onSaved: (state: SetupMediaPathValidationState) => void;
};

/**
 * Checks every listed root can be reached, then saves regardless: a folder
 * that is not mounted yet is a warning, not a reason to stop setup.
 */
export async function runAdvisorySetupMediaPathSave({
  roots,
  validatePath,
  save,
  onValidation,
  onSaved,
}: AdvisorySetupMediaPathSaveOptions): Promise<void> {
  const paths = SETUP_MEDIA_PATH_FIELDS.flatMap((field) =>
    normalizeLibraryRootDrafts(roots[field]).map((root) => root.path),
  );
  const result = await validateLibraryRootPaths(paths, validatePath);
  const state = {
    invalidPaths: result.invalidPaths,
    unavailable: result.unavailable,
  };
  onValidation(state);

  await save();
  onSaved(state);
}
