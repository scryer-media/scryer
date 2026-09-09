import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Client } from "urql";

import { isProwlarrDiscoveryReady } from "@/lib/external-import-wizard-orchestration";
import {
  cancelExternalImportArrSourceWarmupMutation,
  clearExternalImportSetupSecretDraftMutation,
  createLibraryMutation,
  completeSetupMutation,
  executeExternalImportMutation,
  finalizeExternalImportMutation,
  previewExternalImportMutation,
  saveExternalImportSetupSecretDraftMutation,
  scanLibraryMutation,
  startExternalImportArrSourceWarmupMutation,
  startExternalImportProwlarrWarmupMutation,
  updateLibraryMutation,
  validateExternalImportConnectionMutation,
  type ExecuteExternalImportInput,
  type ExternalImportSourceLibraryMappingInput,
} from "@/lib/graphql/mutations";
import {
  browsePathQuery,
  externalImportAggregateWarmupProgressQuery,
  externalImportWarmupStatusQuery,
  externalImportSetupSecretDraftQuery,
  externalImportSetupSecretDraftStatusQuery,
  wizardQualityProfilesQuery,
} from "@/lib/graphql/queries";
import type {
  ExternalArrSourceKind,
  ExternalImportConnectionKind,
  ExternalImportConnectionValidation,
  ExternalImportPreview,
  ExternalImportAggregateWarmupProgress,
  ExternalImportMonitorWarmupProgress,
  ExternalImportResult,
} from "@/lib/types/external-import";

// ── Wizard-internal model ───────────────────────────────────────────────────

export type WizardFacet = "MOVIE" | "SERIES" | "ANIME";
export type ImportArrKind = ExternalArrSourceKind; // "sonarr" | "radarr"
export type ImportInstanceKind = ExternalImportConnectionKind; // + "prowlarr"
export type ImportInstanceStatus = "idle" | "testing" | "connected" | "error";

/** A single Sonarr/Radarr/Prowlarr connection the operator is configuring. */
export interface ImportInstance {
  id: string;
  kind: ImportInstanceKind;
  name: string;
  baseUrl: string;
  apiKey: string;
  status: ImportInstanceStatus;
  version: string | null;
  error: string | null;
  /** Child/title discovery session created after successful connection validation. */
  warmupSessionId: string | null;
}

/**
 * A source root in the mapping board. Detected roots come from a warmup; manual
 * roots are operator-typed and carry no monitored-status snapshot.
 */
export interface ImportRoot {
  id: string;
  kind: ImportArrKind | "manual";
  /** Stable per-source identity (null for manual). */
  sourceKey: string | null;
  sourceWarmupSessionId: string | null;
  /** Short label for the source instance pill, e.g. "Main", "4K", "Manual". */
  instanceLabel: string;
  /** Provenance: the path the source instance reports (or the typed path for manual). */
  arrRootPath: string;
  /** Optional Scryer-host override (detected roots only). Effective path = remap ?? arrRootPath. */
  remap: string | null;
  manual: boolean;
}

/** A library the operator is assembling in the board (created at finalize time). */
export interface ImportLibraryDraft {
  id: string; // client-side temp id until createLibrary returns a real one
  facet: WizardFacet;
  name: string;
  qualityProfileId: string | null;
  scoringPersona: ScoringPersonaValue;
  /** Real backend library id when this draft is an existing library (the
   *  per-facet defaults). null for a user-added library created at finalize. */
  existingLibraryId: string | null;
  /** Default per-facet library — always present and not removable. */
  isDefault: boolean;
}

export type ScoringPersonaValue =
  | "BALANCED"
  | "AUDIOPHILE"
  | "EFFICIENT"
  | "COMPATIBLE";

export const SCORING_PERSONA_VALUES: readonly ScoringPersonaValue[] = [
  "BALANCED",
  "AUDIOPHILE",
  "EFFICIENT",
  "COMPATIBLE",
];

export interface QualityProfileOption {
  id: string;
  name: string;
}

// ── Helpers (pure) ──────────────────────────────────────────────────────────

export function effectiveRootPath(root: ImportRoot): string {
  return (root.remap ?? "").trim() ? (root.remap as string) : root.arrRootPath;
}

export function isRootRemapped(root: ImportRoot): boolean {
  const remap = (root.remap ?? "").trim();
  return remap.length > 0 && remap !== root.arrRootPath;
}

export function facetsForKind(kind: ImportRoot["kind"]): WizardFacet[] {
  switch (kind) {
    case "RADARR":
      return ["MOVIE"];
    case "SONARR":
      return ["SERIES", "ANIME"];
    default:
      return ["MOVIE", "SERIES", "ANIME"]; // manual roots fit any library
  }
}

export function kindCompatibleWithFacet(
  kind: ImportRoot["kind"],
  facet: WizardFacet,
): boolean {
  return facetsForKind(kind).includes(facet);
}

/** Short instance label from a source key like "sonarr:http://host:8989". */
function shortInstanceLabel(instanceName: string, baseUrl: string): string {
  const trimmed = instanceName.trim();
  if (trimmed) return trimmed;
  try {
    const url = new URL(baseUrl);
    return url.port ? `${url.hostname}:${url.port}` : url.hostname;
  } catch {
    return baseUrl;
  }
}

let tempIdSeq = 0;
function tempId(prefix: string): string {
  tempIdSeq += 1;
  const rand =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID().slice(0, 8)
      : String(tempIdSeq);
  return `${prefix}-${rand}-${tempIdSeq}`;
}

function rootIdFor(
  sessionId: string,
  sourceKey: string,
  arrRootPath: string,
): string {
  // Include the warmup session id so the same Arr base URL added as two
  // instances (distinct sessions sharing one sourceKey) yields distinct roots.
  return `src:${sessionId}::${sourceKey}::${arrRootPath}`;
}

function arrKindOf(kind: ImportInstanceKind): ImportArrKind | null {
  return kind === "SONARR" || kind === "RADARR" ? kind : null;
}

function gqlError(error: unknown): string {
  if (!error) return "Unknown error";
  if (error instanceof Error) return error.message;
  const message = (error as { message?: unknown }).message;
  return typeof message === "string" ? message : String(error);
}

/**
 * A warmup session the client still references no longer exists on the server
 * (the orchestrator is in-memory, so a restart or the terminal TTL drops it).
 * This is unrecoverable in place — the only fix is to mint a fresh session by
 * re-verifying the connection, so the wizard routes the user back to Connect.
 */
function isLostWarmupSessionError(message: string | null | undefined): boolean {
  return typeof message === "string" && /no warmup session/i.test(message);
}

// ── Persistence ─────────────────────────────────────────────────────────────
// Non-sensitive wizard state is persisted to sessionStorage so a page refresh
// (or navigating away and back) doesn't discard connections, mappings,
// libraries, or selections. SECRETS (instance API keys, client passwords,
// indexer keys) are NOT stored here — they live in the server-side encrypted
// draft (see the secret-draft sync below). Persisted instances have their
// apiKey stripped; it is re-merged from the server draft on load.
const IMPORT_WIZARD_STORAGE_KEY = "scryer:import-wizard:v1";

interface PersistedImportWizardState {
  instances: ImportInstance[]; // apiKey stripped — restored from the server draft
  manualRoots: ImportRoot[];
  remaps: Record<string, string>;
  assign: Record<string, string | null>;
  libraries: ImportLibraryDraft[];
  selectedDcKeys: string[];
  selectedIdxKeys: string[];
  dcSelectionSeeded: boolean;
  idxSelectionSeeded: boolean;
  executeResult: ExternalImportResult | null;
}

function loadPersistedImportWizardState(): Partial<PersistedImportWizardState> | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem(IMPORT_WIZARD_STORAGE_KEY);
    return raw
      ? (JSON.parse(raw) as Partial<PersistedImportWizardState>)
      : null;
  } catch {
    return null;
  }
}

function savePersistedImportWizardState(
  state: PersistedImportWizardState,
): void {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.setItem(
      IMPORT_WIZARD_STORAGE_KEY,
      JSON.stringify(state),
    );
  } catch {
    // best-effort: sessionStorage may be unavailable or full.
  }
}

function clearPersistedImportWizardState(): void {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.removeItem(IMPORT_WIZARD_STORAGE_KEY);
  } catch {
    // ignore
  }
}

interface UseExternalImportSetupArgs {
  client: Client;
}

export function useExternalImportSetup({ client }: UseExternalImportSetupArgs) {
  // Snapshot of any persisted state, read once on mount to hydrate the wizard.
  const initial = useMemo(loadPersistedImportWizardState, []);

  // ── Connect step ──────────────────────────────────────────────────────────
  const [instances, setInstances] = useState<ImportInstance[]>(
    () => initial?.instances ?? [],
  );
  // Last successfully-verified {baseUrl, apiKey} per instance, so an incidental
  // re-blur of an unchanged connected instance doesn't discard its finished
  // warmup and re-run the full library fetch.
  const lastVerifiedRef = useRef<
    Record<string, { baseUrl: string; apiKey: string }>
  >({});
  // Libraries created during a finalize attempt, persisted across retries so a
  // resumed finalize doesn't re-create (and conflict on) existing libraries.
  const createdLibrariesRef = useRef<Map<string, string>>(new Map());

  const arrInstances = useMemo(
    () => instances.filter((inst) => inst.kind !== "PROWLARR"),
    [instances],
  );
  const prowlarrInstance = useMemo(
    () => instances.find((inst) => inst.kind === "PROWLARR") ?? null,
    [instances],
  );

  const instancesByKind = useCallback(
    (kind: ImportInstanceKind) => instances.filter((inst) => inst.kind === kind),
    [instances],
  );

  const patchInstance = useCallback(
    (id: string, patch: Partial<ImportInstance>) => {
      setInstances((prev) =>
        prev.map((inst) => (inst.id === id ? { ...inst, ...patch } : inst)),
      );
    },
    [],
  );

  const addInstance = useCallback((kind: ImportInstanceKind) => {
    setInstances((prev) => [
      ...prev,
      {
        id: tempId(kind),
        kind,
        name: "",
        baseUrl: "",
        apiKey: "",
        status: "idle",
        version: null,
        error: null,
        warmupSessionId: null,
      },
    ]);
  }, []);

  const cancelInstanceWarmup = useCallback(
    (sessionId: string) => {
      void client
        .mutation(cancelExternalImportArrSourceWarmupMutation, { sessionId })
        .toPromise();
    },
    [client],
  );

  const removeInstance = useCallback(
    (id: string) => {
      setInstances((prev) => {
        const target = prev.find((inst) => inst.id === id);
        if (target?.warmupSessionId) cancelInstanceWarmup(target.warmupSessionId);
        return prev.filter((inst) => inst.id !== id);
      });
    },
    [cancelInstanceWarmup],
  );

  const setInstanceName = useCallback(
    (id: string, name: string) => patchInstance(id, { name }),
    [patchInstance],
  );

  /** Edit URL/key: reset verification state and tear down any prior warmup. */
  const setInstanceConnectionField = useCallback(
    (id: string, field: "baseUrl" | "apiKey", value: string) => {
      setInstances((prev) =>
        prev.map((inst) => {
          if (inst.id !== id) return inst;
          // A connection edit invalidates the prior verify/warmup; cancel it so
          // it isn't orphaned on the backend.
          if (inst.warmupSessionId) cancelInstanceWarmup(inst.warmupSessionId);
          return {
            ...inst,
            ...(field === "baseUrl" ? { baseUrl: value } : { apiKey: value }),
            status: "idle" as const,
            version: null,
            error: null,
            warmupSessionId: null,
          };
        }),
      );
    },
    [cancelInstanceWarmup],
  );

  const connectionReady = (inst: ImportInstance): boolean =>
    /^https?:\/\/.+/.test(inst.baseUrl.trim()) && inst.apiKey.trim().length >= 6;

  /**
   * Fire-on-blur verification. Validates connectivity via the lightweight probe,
   * and on success (for Sonarr/Radarr) kicks off the warmup in the background so
   * warmups run concurrently while the operator keeps adding instances.
   */
  const verifyInstance = useCallback(
    async (id: string) => {
      const inst = instances.find((entry) => entry.id === id);
      if (!inst || !connectionReady(inst)) return;
      const connection = {
        baseUrl: inst.baseUrl.trim(),
        apiKey: inst.apiKey.trim(),
      };
      // Skip a redundant re-verify of an already-connected, unchanged instance
      // (re-blur) — re-running would evict a finished warmup and re-fetch.
      const lastVerified = lastVerifiedRef.current[id];
      if (
        inst.status === "connected" &&
        lastVerified &&
        lastVerified.baseUrl === connection.baseUrl &&
        lastVerified.apiKey === connection.apiKey
      ) {
        return;
      }
      patchInstance(id, { status: "testing", error: null });

      const { data, error } = await client
        .mutation(validateExternalImportConnectionMutation, {
          input: { kind: inst.kind, connection },
        })
        .toPromise();

      const validation = data?.validateExternalImportConnection as
        | ExternalImportConnectionValidation
        | undefined;
      if (error || !validation) {
        patchInstance(id, {
          status: "error",
          version: null,
          error: gqlError(error) || "Connection failed",
        });
        return;
      }
      if (!validation.connected) {
        patchInstance(id, {
          status: "error",
          version: null,
          error: validation.error ?? "Could not connect",
        });
        return;
      }

      patchInstance(id, {
        status: "connected",
        version: validation.version ?? null,
        error: null,
      });
      lastVerifiedRef.current[id] = connection;

      // Kick off child/title discovery without holding the validation request.
      const arrKind = arrKindOf(inst.kind);
      const { data: warmupData, error: warmupError } = await client
        .mutation(
          arrKind
            ? startExternalImportArrSourceWarmupMutation
            : startExternalImportProwlarrWarmupMutation,
          {
            input: arrKind ? { kind: arrKind, connection } : { connection },
          },
        )
        .toPromise();
      const sessionId = (arrKind
        ? warmupData?.startExternalImportArrSourceWarmup?.sessionId
        : warmupData?.startExternalImportProwlarrWarmup?.sessionId) as
        | string
        | undefined;
      if (!arrKind && !sessionId) {
        patchInstance(id, {
          status: "error",
          error:
            gqlError(warmupError) ||
            "Connection succeeded, but Prowlarr discovery could not start",
        });
        return;
      }
      if (sessionId) {
          // Reconcile the freshly-started session against the latest state,
          // cancelling any session that is now orphaned. Orphans starve the
          // backend's scarce episode-warmup slots and leave the live warmup
          // stuck at "loading episodes". Two orphan sources are handled here:
          //   1. The instance was removed, or its connection changed while the
          //      warmup was starting → the new session is for a stale config.
          //   2. The instance already had a different session (a racing
          //      re-verify the backend didn't dedup) → supersede the old one.
          let toCancel: string | null = null;
          setInstances((prev) => {
            const current = prev.find((entry) => entry.id === id);
            if (
              !current ||
              current.baseUrl.trim() !== connection.baseUrl ||
              current.apiKey.trim() !== connection.apiKey
            ) {
              toCancel = sessionId;
              return prev;
            }
            if (current.warmupSessionId && current.warmupSessionId !== sessionId) {
              toCancel = current.warmupSessionId;
            }
            return prev.map((entry) =>
              entry.id === id ? { ...entry, warmupSessionId: sessionId } : entry,
            );
          });
          if (toCancel) cancelInstanceWarmup(toCancel);
        }
    },
    [client, instances, patchInstance, cancelInstanceWarmup],
  );

  const connectedArrSessionIds = useMemo(
    () =>
      arrInstances
        .filter((inst) => inst.status === "connected" && inst.warmupSessionId)
        .map((inst) => inst.warmupSessionId as string),
    [arrInstances],
  );
  const prowlarrWarmupSessionId =
    prowlarrInstance?.status === "connected"
      ? prowlarrInstance.warmupSessionId
      : null;

  /** ≥1 connected Sonarr/Radarr (or a connected Prowlarr) is required to advance. */
  const canLeaveConnect = useMemo(
    () => instances.some((inst) => inst.status === "connected"),
    [instances],
  );

  const prowlarrConnectionInput = useMemo(() => {
    if (!prowlarrInstance || prowlarrInstance.status !== "connected") return null;
    return {
      baseUrl: prowlarrInstance.baseUrl.trim(),
      apiKey: prowlarrInstance.apiKey.trim(),
    };
  }, [prowlarrInstance]);

  // ── Preview (root folders, clients, indexers) ──────────────────────────────
  const [preview, setPreview] = useState<ExternalImportPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [prowlarrWarmupProgress, setProwlarrWarmupProgress] =
    useState<ExternalImportMonitorWarmupProgress | null>(null);
  const prowlarrWarmupReady = isProwlarrDiscoveryReady(
    Boolean(prowlarrConnectionInput),
    prowlarrWarmupSessionId,
    prowlarrWarmupProgress?.status ?? null,
  );
  const [previewError, setPreviewError] = useState<string | null>(null);
  // A referenced warmup session vanished server-side (restart / TTL). Unlike a
  // transient failure this can't be retried in place — recovery is to re-verify
  // on Connect, so the UI offers "Reconnect" instead of "Retry".
  const [warmupSessionLost, setWarmupSessionLost] = useState(false);
  // Set during lost-session recovery: tells the Connect step to auto-verify the
  // restored connections once (so a fresh session starts without the operator
  // having to manually re-blur each field).
  const [pendingReverify, setPendingReverify] = useState(false);

  const loadPreview = useCallback(async () => {
    if (connectedArrSessionIds.length === 0 && !prowlarrWarmupSessionId) {
      setPreview(null);
      return;
    }
    setPreviewing(true);
    setPreviewError(null);
    const { data, error } = await client
      .mutation(previewExternalImportMutation, {
        input: {
          sourceWarmupSessionIds: connectedArrSessionIds,
          prowlarrWarmupSessionId,
        },
      })
      .toPromise();
    setPreviewing(false);
    if (error || !data?.previewExternalImport) {
      const message = gqlError(error) || "Failed to load preview";
      setPreviewError(message);
      if (isLostWarmupSessionError(message)) setWarmupSessionLost(true);
      return;
    }
    setPreview(data.previewExternalImport as ExternalImportPreview);
    setWarmupSessionLost(false);
  }, [client, connectedArrSessionIds, prowlarrWarmupSessionId]);

  useEffect(() => {
    if (!prowlarrWarmupSessionId) {
      setProwlarrWarmupProgress(null);
      return;
    }

    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    // Transient failures back off exponentially (capped) instead of hammering
    // the status endpoint at the base cadence; a successful poll resets it.
    const basePollMs = 750;
    const maxPollMs = 5000;
    let errorPollMs = basePollMs;
    const poll = async () => {
      const { data, error } = await client
        .query(
          externalImportWarmupStatusQuery,
          { sessionId: prowlarrWarmupSessionId },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (stopped) return;
      if (error || !data?.externalImportWarmupStatus) {
        const message = gqlError(error) || "Failed to load Prowlarr discovery status";
        setPreviewError(message);
        if (isLostWarmupSessionError(message)) {
          setWarmupSessionLost(true);
        } else {
          timer = setTimeout(poll, errorPollMs);
          errorPollMs = Math.min(errorPollMs * 2, maxPollMs);
        }
        return;
      }
      errorPollMs = basePollMs;

      const progress =
        data.externalImportWarmupStatus as ExternalImportMonitorWarmupProgress;
      setProwlarrWarmupProgress(progress);
      if (progress.status === "COMPLETED") {
        void loadPreview();
      } else if (progress.status === "QUEUED" || progress.status === "RUNNING") {
        timer = setTimeout(poll, basePollMs);
      }
    };

    void poll();
    return () => {
      stopped = true;
      if (timer) clearTimeout(timer);
    };
  }, [client, loadPreview, prowlarrWarmupSessionId]);

  const retryProwlarrWarmup = useCallback(async () => {
    if (!prowlarrConnectionInput || !prowlarrInstance) return;
    setPreviewError(null);
    setProwlarrWarmupProgress(null);
    const { data, error } = await client
      .mutation(startExternalImportProwlarrWarmupMutation, {
        input: { connection: prowlarrConnectionInput },
      })
      .toPromise();
    const sessionId = data?.startExternalImportProwlarrWarmup?.sessionId as
      | string
      | undefined;
    if (error || !sessionId) {
      setPreviewError(gqlError(error) || "Failed to restart Prowlarr discovery");
      return;
    }
    setInstances((prev) =>
      prev.map((instance) =>
        instance.id === prowlarrInstance.id
          ? { ...instance, warmupSessionId: sessionId }
          : instance,
      ),
    );
  }, [client, prowlarrConnectionInput, prowlarrInstance]);

  // ── Mapping board: roots + manual roots + remaps + assign ──────────────────
  const [manualRoots, setManualRoots] = useState<ImportRoot[]>(
    () => initial?.manualRoots ?? [],
  );
  const [remaps, setRemaps] = useState<Record<string, string>>(
    () => initial?.remaps ?? {},
  );
  const [assign, setAssign] = useState<Record<string, string | null>>(
    () => initial?.assign ?? {},
  );
  const [invalidAssignedRootIds, setInvalidAssignedRootIds] = useState<
    string[]
  >([]);
  const [validatedAssignedRootPathSnapshotKey, setValidatedAssignedRootPathSnapshotKey] =
    useState<string | null>(null);

  const detectedRoots = useMemo<ImportRoot[]>(() => {
    if (!preview) return [];
    return preview.rootFolders.map((folder) => {
      const id = rootIdFor(
        folder.sourceWarmupSessionId,
        folder.sourceKey,
        folder.arrRootPath,
      );
      const source = preview.arrSources.find(
        (s) => s.sourceKey === folder.sourceKey,
      );
      return {
        id,
        kind: folder.kind,
        sourceKey: folder.sourceKey,
        sourceWarmupSessionId: folder.sourceWarmupSessionId,
        instanceLabel: shortInstanceLabel("", source?.baseUrl ?? folder.sourceKey),
        arrRootPath: folder.arrRootPath,
        remap: remaps[id] ?? null,
        manual: false,
      };
    });
  }, [preview, remaps]);

  const roots = useMemo<ImportRoot[]>(
    () => [...detectedRoots, ...manualRoots],
    [detectedRoots, manualRoots],
  );

  const rootById = useCallback(
    (id: string) => roots.find((root) => root.id === id) ?? null,
    [roots],
  );

  const trayRoots = useMemo(
    () => roots.filter((root) => !assign[root.id]),
    [roots, assign],
  );

  const rootsForLibrary = useCallback(
    (libraryId: string) => roots.filter((root) => assign[root.id] === libraryId),
    [roots, assign],
  );

  const assignRoot = useCallback((rootId: string, libraryId: string | null) => {
    setAssign((prev) => ({ ...prev, [rootId]: libraryId }));
  }, []);

  const assignedRootPathValidationSnapshot = useMemo(() => {
    const assignedRoots = roots
      .filter((root) => Boolean(assign[root.id]))
      .map((root) => ({
        id: root.id,
        path: effectiveRootPath(root).trim(),
      }))
      .filter((root) => root.path.length > 0);

    const key = assignedRoots
      .map((root) => `${root.id}:${root.path}`)
      .sort()
      .join("\u001e");

    return { key, assignedRoots };
  }, [assign, roots]);

  const assignedRootPathValidationLoading =
    assignedRootPathValidationSnapshot.assignedRoots.length > 0 &&
    validatedAssignedRootPathSnapshotKey !==
      assignedRootPathValidationSnapshot.key;

  useEffect(() => {
    const { key, assignedRoots } = assignedRootPathValidationSnapshot;

    if (assignedRoots.length === 0) {
      setInvalidAssignedRootIds([]);
      setValidatedAssignedRootPathSnapshotKey(key);
      return;
    }

    let cancelled = false;

    const validateAssignedRoots = async () => {
      const invalidIds = new Set<string>();

      await Promise.all(
        assignedRoots.map(async (root) => {
          const { error } = await client
            .query(
              browsePathQuery,
              { path: root.path },
              { requestPolicy: "network-only" },
            )
            .toPromise();

          if (error) {
            invalidIds.add(root.id);
          }
        }),
      );

      if (!cancelled) {
        setInvalidAssignedRootIds([...invalidIds]);
        setValidatedAssignedRootPathSnapshotKey(key);
      }
    };

    void validateAssignedRoots().catch((error) => {
      console.error(
        "[external-import-root-validation] failed to validate assigned roots:",
        error,
      );
      if (!cancelled) {
        setInvalidAssignedRootIds([]);
        setValidatedAssignedRootPathSnapshotKey(key);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [assignedRootPathValidationSnapshot, client]);

  const setRootRemap = useCallback((rootId: string, scryerPath: string | null) => {
    setRemaps((prev) => {
      const next = { ...prev };
      const value = (scryerPath ?? "").trim();
      if (!value) delete next[rootId];
      else next[rootId] = value;
      return next;
    });
  }, []);

  // Manual roots are added via the folder browser (no free typing), so the path
  // is supplied up front.
  const addManualRoot = useCallback((path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    const id = tempId("manual-root");
    setManualRoots((prev) => [
      ...prev,
      {
        id,
        kind: "manual",
        sourceKey: null,
        sourceWarmupSessionId: null,
        instanceLabel: "Manual",
        arrRootPath: trimmed,
        remap: null,
        manual: true,
      },
    ]);
  }, []);

  const setManualRootPath = useCallback((rootId: string, path: string) => {
    setManualRoots((prev) =>
      prev.map((root) =>
        root.id === rootId ? { ...root, arrRootPath: path } : root,
      ),
    );
  }, []);

  const removeManualRoot = useCallback((rootId: string) => {
    setManualRoots((prev) => prev.filter((root) => root.id !== rootId));
    setAssign((prev) => {
      const next = { ...prev };
      delete next[rootId];
      return next;
    });
  }, []);

  // ── Library drafts ─────────────────────────────────────────────────────────
  // The board always starts with the three per-facet default libraries
  // (Movies / Series / Anime), which are not removable. Their `existingLibraryId`
  // is the deterministic default id; at finalize each is updated in place if it
  // already exists, otherwise created. The operator maps roots into them and can
  // add more libraries alongside.
  const [libraries, setLibraries] = useState<ImportLibraryDraft[]>(
    () => initial?.libraries ?? defaultLibraryDrafts(),
  );

  const addLibrary = useCallback((facet: WizardFacet, name?: string) => {
    const id = tempId("lib");
    setLibraries((prev) => [
      ...prev,
      {
        id,
        facet,
        name: name?.trim() || defaultLibraryName(facet, prev),
        qualityProfileId: null,
        scoringPersona: "BALANCED",
        existingLibraryId: null,
        isDefault: false,
      },
    ]);
    return id;
  }, []);

  const renameLibrary = useCallback((id: string, name: string) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    setLibraries((prev) =>
      prev.map((lib) => (lib.id === id ? { ...lib, name: trimmed } : lib)),
    );
  }, []);

  const removeLibrary = useCallback(
    (id: string) => {
      const target = libraries.find((lib) => lib.id === id);
      if (!target || target.isDefault) return; // defaults are not removable
      setLibraries((prev) => prev.filter((lib) => lib.id !== id));
      setAssign((prev) => {
        const next = { ...prev };
        for (const rootId of Object.keys(next)) {
          if (next[rootId] === id) next[rootId] = null;
        }
        return next;
      });
    },
    [libraries],
  );

  const setLibraryQualityProfile = useCallback(
    (id: string, qualityProfileId: string | null) => {
      setLibraries((prev) =>
        prev.map((lib) =>
          lib.id === id ? { ...lib, qualityProfileId } : lib,
        ),
      );
    },
    [],
  );

  const setLibraryPersona = useCallback(
    (id: string, scoringPersona: ScoringPersonaValue) => {
      setLibraries((prev) =>
        prev.map((lib) => (lib.id === id ? { ...lib, scoringPersona } : lib)),
      );
    },
    [],
  );

  /** Libraries that have at least one root mapped (shown in Quality step). */
  const mappedLibraries = useMemo(
    () =>
      libraries.filter((lib) =>
        roots.some((root) => assign[root.id] === lib.id),
      ),
    [libraries, roots, assign],
  );

  // ── Quality profiles ───────────────────────────────────────────────────────
  const [qualityProfiles, setQualityProfiles] = useState<QualityProfileOption[]>(
    [],
  );
  useEffect(() => {
    let cancelled = false;
    void client
      .query(wizardQualityProfilesQuery, {})
      .toPromise()
      .then(({ data }) => {
        if (cancelled) return;
        const profiles = (data?.qualityProfileSettings?.profiles ??
          []) as QualityProfileOption[];
        setQualityProfiles(profiles);
      });
    return () => {
      cancelled = true;
    };
  }, [client]);

  // Every library needs a quality profile — there is no "none" option. As soon
  // as profiles are available, default any library without a selection to the
  // first profile. Reacting to `libraries` too covers user-added libraries.
  useEffect(() => {
    if (qualityProfiles.length === 0) return;
    if (!libraries.some((lib) => !lib.qualityProfileId)) return;
    const fallback = qualityProfiles[0].id;
    setLibraries((prev) =>
      prev.map((lib) =>
        lib.qualityProfileId ? lib : { ...lib, qualityProfileId: fallback },
      ),
    );
  }, [qualityProfiles, libraries]);

  // The Quality step can't be left until every mapped library has a profile.
  const qualityReady = useMemo(
    () => mappedLibraries.every((lib) => Boolean(lib.qualityProfileId)),
    [mappedLibraries],
  );

  // ── Sources step: download client + indexer selection ──────────────────────
  const [selectedDcKeys, setSelectedDcKeys] = useState<Set<string>>(
    () => new Set(initial?.selectedDcKeys ?? []),
  );
  const [selectedIdxKeys, setSelectedIdxKeys] = useState<Set<string>>(
    () => new Set(initial?.selectedIdxKeys ?? []),
  );
  // Secret override maps are NOT persisted to sessionStorage — they are synced
  // to the server-side encrypted draft and re-hydrated from it on load.
  const [dcApiKeyOverrides, setDcApiKeyOverrides] = useState<
    Record<string, string>
  >({});
  const [dcPasswordOverrides, setDcPasswordOverrides] = useState<
    Record<string, string>
  >({});
  const [idxApiKeyOverrides, setIdxApiKeyOverrides] = useState<
    Record<string, string>
  >({});
  // Restore the "defaults seeded" flags so a refresh after the operator
  // customized selections doesn't re-seed everything back on.
  const dcSelectionSeeded = useRef(initial?.dcSelectionSeeded ?? false);
  const idxSelectionSeeded = useRef(initial?.idxSelectionSeeded ?? false);

  // ── Server-side secret draft sync ──────────────────────────────────────────
  // API keys / passwords are stored server-side (encrypted, owner-scoped
  // singleton), never in sessionStorage. Hydrate from it on mount, debounce-save
  // when secrets change, and clear it on finalize. `secretsHydrated` gates the
  // save effect so it can't wipe the draft before the initial load completes.
  const [secretDraftOwnedByOther, setSecretDraftOwnedByOther] = useState(false);
  const [secretDraftOverwroteOther, setSecretDraftOverwroteOther] =
    useState(false);
  const [secretsHydrated, setSecretsHydrated] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const { data: statusData } = await client
          .query(
            externalImportSetupSecretDraftStatusQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise();
        const status = statusData?.externalImportSetupSecretDraftStatus;
        if (cancelled) return;
        if (status?.hasDraft && !status.ownedByCurrentUser) {
          setSecretDraftOwnedByOther(true);
        }
        if (status?.hasDraft && status.ownedByCurrentUser) {
          const { data } = await client
            .query(
              externalImportSetupSecretDraftQuery,
              {},
              { requestPolicy: "network-only" },
            )
            .toPromise();
          const draft = data?.externalImportSetupSecretDraft;
          if (!cancelled && draft) {
            const keyByInstanceId = new Map<string, string>(
              (
                draft.instanceApiKeys as Array<{
                  instanceId: string;
                  apiKey: string;
                }>
              ).map((entry) => [entry.instanceId, entry.apiKey]),
            );
            setInstances((prev) =>
              prev.map((inst) =>
                keyByInstanceId.has(inst.id)
                  ? { ...inst, apiKey: keyByInstanceId.get(inst.id) ?? "" }
                  : inst,
              ),
            );
            const toRecord = (
              entries: Array<Record<string, string>>,
              valueKey: string,
            ): Record<string, string> =>
              Object.fromEntries(
                entries.map((e) => [e.dedupKey, e[valueKey] ?? ""]),
              );
            setDcApiKeyOverrides(
              toRecord(draft.downloadClientApiKeyOverrides ?? [], "apiKey"),
            );
            setDcPasswordOverrides(
              toRecord(draft.downloadClientPasswordOverrides ?? [], "password"),
            );
            setIdxApiKeyOverrides(
              toRecord(draft.indexerApiKeyOverrides ?? [], "apiKey"),
            );
          }
        }
      } catch {
        // best-effort: proceed without restored secrets.
      } finally {
        if (!cancelled) setSecretsHydrated(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client]);

  // Debounced save of secrets to the server draft. Gated on `secretsHydrated`
  // so it can't wipe the draft before the initial load. Empty payload -> clear
  // (the backend rejects empty saves).
  useEffect(() => {
    if (!secretsHydrated) return;
    const instanceApiKeys = instances
      .filter((inst) => inst.apiKey.trim())
      .map((inst) => ({
        instanceId: inst.id,
        kind: inst.kind,
        apiKey: inst.apiKey.trim(),
      }));
    const downloadClientApiKeyOverrides = Object.entries(dcApiKeyOverrides)
      .filter(([, v]) => v.trim())
      .map(([dedupKey, apiKey]) => ({ dedupKey, apiKey: apiKey.trim() }));
    const downloadClientPasswordOverrides = Object.entries(dcPasswordOverrides)
      .filter(([, v]) => v.trim())
      .map(([dedupKey, password]) => ({ dedupKey, password: password.trim() }));
    const indexerApiKeyOverrides = Object.entries(idxApiKeyOverrides)
      .filter(([, v]) => v.trim())
      .map(([dedupKey, apiKey]) => ({ dedupKey, apiKey: apiKey.trim() }));
    const hasAny =
      instanceApiKeys.length > 0 ||
      downloadClientApiKeyOverrides.length > 0 ||
      downloadClientPasswordOverrides.length > 0 ||
      indexerApiKeyOverrides.length > 0;
    const timer = setTimeout(() => {
      if (hasAny) {
        void client
          .mutation(saveExternalImportSetupSecretDraftMutation, {
            input: {
              instanceApiKeys,
              downloadClientApiKeyOverrides,
              downloadClientPasswordOverrides,
              indexerApiKeyOverrides,
            },
          })
          .toPromise()
          .then(({ data }) => {
            if (
              data?.saveExternalImportSetupSecretDraft?.overwroteAnotherUserDraft
            ) {
              setSecretDraftOverwroteOther(true);
              setSecretDraftOwnedByOther(false);
            }
          });
      } else {
        void client
          .mutation(clearExternalImportSetupSecretDraftMutation, {})
          .toPromise();
      }
    }, 800);
    return () => clearTimeout(timer);
  }, [
    secretsHydrated,
    instances,
    dcApiKeyOverrides,
    dcPasswordOverrides,
    idxApiKeyOverrides,
    client,
  ]);

  // Default all supported clients/indexers ON the first time a preview arrives.
  useEffect(() => {
    if (!preview) return;
    if (!dcSelectionSeeded.current) {
      dcSelectionSeeded.current = true;
      setSelectedDcKeys(
        new Set(
          preview.downloadClients
            .filter((dc) => dc.supported)
            .map((dc) => dc.dedupKey),
        ),
      );
    }
    if (!idxSelectionSeeded.current) {
      idxSelectionSeeded.current = true;
      setSelectedIdxKeys(
        new Set(
          preview.indexers
            .filter((idx) => idx.supported)
            .map((idx) => idx.dedupKey),
        ),
      );
    }
  }, [preview]);

  const toggleDownloadClient = useCallback((dedupKey: string) => {
    setSelectedDcKeys((prev) => {
      const next = new Set(prev);
      if (next.has(dedupKey)) next.delete(dedupKey);
      else next.add(dedupKey);
      return next;
    });
  }, []);

  const toggleIndexer = useCallback((dedupKey: string) => {
    setSelectedIdxKeys((prev) => {
      const next = new Set(prev);
      if (next.has(dedupKey)) next.delete(dedupKey);
      else next.add(dedupKey);
      return next;
    });
  }, []);

  const setDownloadClientApiKeyOverride = useCallback(
    (dedupKey: string, value: string) =>
      setDcApiKeyOverrides((prev) => ({ ...prev, [dedupKey]: value })),
    [],
  );
  const setDownloadClientPasswordOverride = useCallback(
    (dedupKey: string, value: string) =>
      setDcPasswordOverrides((prev) => ({ ...prev, [dedupKey]: value })),
    [],
  );
  const setIndexerApiKeyOverride = useCallback(
    (dedupKey: string, value: string) =>
      setIdxApiKeyOverrides((prev) => ({ ...prev, [dedupKey]: value })),
    [],
  );

  const [executing, setExecuting] = useState(false);
  const [executeError, setExecuteError] = useState<string | null>(null);
  const [executeResult, setExecuteResult] =
    useState<ExternalImportResult | null>(() => initial?.executeResult ?? null);

  const executeSources = useCallback(async (): Promise<{
    ok: boolean;
    error: string | null;
  }> => {
    // Idempotent: clients/indexers are created exactly once per wizard session.
    // Re-entering Sources (back→forward) must not duplicate them — the backend
    // create path does not dedup — so short-circuit once a prior run succeeded.
    if (executeResult) return { ok: true, error: null };
    if (!prowlarrWarmupReady) {
      const error =
        prowlarrWarmupProgress?.errorMessage ||
        "Prowlarr indexer discovery must finish before importing sources.";
      setExecuteError(error);
      return { ok: false, error };
    }
    setExecuting(true);
    setExecuteError(null);
    const input: ExecuteExternalImportInput = {
      sourceWarmupSessionIds: connectedArrSessionIds,
      prowlarr: prowlarrConnectionInput,
      selectedDownloadClientDedupKeys: [...selectedDcKeys],
      selectedIndexerDedupKeys: [...selectedIdxKeys],
      downloadClientApiKeyOverrides: Object.entries(dcApiKeyOverrides)
        .filter(([, v]) => v.trim())
        .map(([dedupKey, apiKey]) => ({ dedupKey, apiKey })),
      downloadClientPasswordOverrides: Object.entries(dcPasswordOverrides)
        .filter(([, v]) => v.trim())
        .map(([dedupKey, password]) => ({ dedupKey, password })),
      indexerApiKeyOverrides: Object.entries(idxApiKeyOverrides)
        .filter(([, v]) => v.trim())
        .map(([dedupKey, apiKey]) => ({ dedupKey, apiKey })),
    };
    const { data, error } = await client
      .mutation(executeExternalImportMutation, { input })
      .toPromise();
    setExecuting(false);
    if (error || !data?.executeExternalImport) {
      const message = gqlError(error) || "Import failed";
      setExecuteError(message);
      return { ok: false, error: message };
    }
    setExecuteResult(data.executeExternalImport as ExternalImportResult);
    return { ok: true, error: null };
  }, [
    client,
    connectedArrSessionIds,
    prowlarrConnectionInput,
    selectedDcKeys,
    selectedIdxKeys,
    dcApiKeyOverrides,
    dcPasswordOverrides,
    idxApiKeyOverrides,
    executeResult,
    prowlarrWarmupReady,
    prowlarrWarmupProgress,
  ]);

  // ── Aggregate warmup progress (gates Summary) ──────────────────────────────
  const [aggregateProgress, setAggregateProgress] =
    useState<ExternalImportAggregateWarmupProgress | null>(null);
  // Separate from a backend "failed" status: set when the progress query itself
  // fails (e.g. a pruned/expired session returns NotFound). Without surfacing
  // this, the Summary spinner would spin forever on a dead session.
  const [aggregateProgressError, setAggregateProgressError] = useState<
    string | null
  >(null);

  const pollAggregateProgress = useCallback(async () => {
    if (connectedArrSessionIds.length === 0) {
      setAggregateProgressError(null);
      setAggregateProgress({
        status: "COMPLETED",
        titlesTotalKnown: true,
        titlesFetched: 0,
        titlesTotal: 0,
        errorMessage: null,
      });
      return;
    }
    const { data, error } = await client
      .query(
        externalImportAggregateWarmupProgressQuery,
        { input: { sourceWarmupSessionIds: connectedArrSessionIds } },
        { requestPolicy: "network-only" },
      )
      .toPromise();
    const progress = data?.externalImportAggregateWarmupProgress as
      | ExternalImportAggregateWarmupProgress
      | undefined;
    if (progress) {
      setAggregateProgress(progress);
      setAggregateProgressError(null);
      setWarmupSessionLost(false);
    } else if (error) {
      const message = gqlError(error) || "Failed to load warmup progress";
      setAggregateProgressError(message);
      if (isLostWarmupSessionError(message)) setWarmupSessionLost(true);
    }
  }, [client, connectedArrSessionIds]);

  const warmupComplete = aggregateProgress?.status === "COMPLETED";
  // The warmup failed (backend reported failed/canceled, or the progress query
  // itself errored with no live progress to fall back on).
  const warmupFailed =
    aggregateProgress?.status === "FAILED" ||
    aggregateProgress?.status === "CANCELED" ||
    (aggregateProgressError !== null && aggregateProgress === null);
  // Terminal either way — used to stop the Summary poll loop.
  const warmupSettled = warmupComplete || warmupFailed;
  const warmupErrorMessage =
    aggregateProgress?.errorMessage ?? aggregateProgressError;

  /**
   * Re-start the per-instance warmups from the Summary step after a failure
   * (failed/canceled status, or a pruned session). Clears the prior progress so
   * the Summary poll picks up the freshly-created sessions.
   */
  const retryWarmup = useCallback(async () => {
    setAggregateProgressError(null);
    setAggregateProgress(null);
    const targets = arrInstances.filter(
      (inst) => inst.status === "connected" && inst.apiKey.trim().length > 0,
    );
    const started = await Promise.all(
      targets.map(async (inst) => {
        const arrKind = arrKindOf(inst.kind);
        if (!arrKind) return false;
        const connection = {
          baseUrl: inst.baseUrl.trim(),
          apiKey: inst.apiKey.trim(),
        };
        const { data } = await client
          .mutation(startExternalImportArrSourceWarmupMutation, {
            input: { kind: arrKind, connection },
          })
          .toPromise();
        const sessionId = data?.startExternalImportArrSourceWarmup?.sessionId as
          | string
          | undefined;
        if (!sessionId) return false;
        let toCancel: string | null = null;
        setInstances((prev) => {
          const current = prev.find((entry) => entry.id === inst.id);
          if (!current) {
            toCancel = sessionId;
            return prev;
          }
          if (current.warmupSessionId && current.warmupSessionId !== sessionId) {
            toCancel = current.warmupSessionId;
          }
          return prev.map((entry) =>
            entry.id === inst.id
              ? { ...entry, warmupSessionId: sessionId }
              : entry,
          );
        });
        if (toCancel) cancelInstanceWarmup(toCancel);
        return true;
      }),
    );
    // If no instance produced a fresh session (every re-warm mutation failed, or
    // there was nothing to restart), surface an error so the Summary keeps its
    // failed + Retry state instead of freezing on a 0% spinner with no poll
    // (connectedArrSessionIds wouldn't change, so the poll would never resume).
    if (!started.some(Boolean)) {
      setAggregateProgressError(
        "Couldn’t restart the monitored-status sync. Check your connections and try again.",
      );
    }
  }, [arrInstances, client, cancelInstanceWarmup]);

  /**
   * Recover from a lost warmup session: the referenced session is gone and can't
   * be re-fetched, so reset every instance to unverified and drop all
   * session-derived state. The caller then routes back to Connect, where the
   * pending-reverify flag triggers a one-shot auto-verify that mints fresh
   * sessions (the verify skip-guard only skips already-"connected" instances and
   * keys off lastVerifiedRef, both cleared here).
   */
  const recoverLostWarmup = useCallback(() => {
    lastVerifiedRef.current = {};
    setInstances((prev) =>
      prev.map((inst) => ({
        ...inst,
        status: "idle" as const,
        warmupSessionId: null,
        version: null,
        error: null,
      })),
    );
    setPreview(null);
    setPreviewError(null);
    setAggregateProgress(null);
    setAggregateProgressError(null);
    setWarmupSessionLost(false);
    setPendingReverify(true);
  }, []);

  const clearPendingReverify = useCallback(() => setPendingReverify(false), []);

  // ── Finalize → complete → scan ─────────────────────────────────────────────
  const buildMappings = useCallback((): {
    mappings: ExternalImportSourceLibraryMappingInput[];
    librariesToCreate: {
      draft: ImportLibraryDraft;
      rootPaths: string[];
    }[];
  } => {
    const librariesToCreate = libraries
      .map((draft) => {
        const assignedRoots = roots.filter((root) => assign[root.id] === draft.id);
        const rootPaths = Array.from(
          new Set(assignedRoots.map((root) => effectiveRootPath(root).trim())),
        ).filter(Boolean);
        return { draft, rootPaths };
      })
      .filter((entry) => entry.rootPaths.length > 0);

    // Mappings are filled in after libraries are created (need real ids).
    return { mappings: [], librariesToCreate };
  }, [libraries, roots, assign]);

  const [finalizing, setFinalizing] = useState(false);
  const [finalizeError, setFinalizeError] = useState<string | null>(null);

  /**
   * Creates the mapped libraries, applies the monitored-status mappings, marks
   * setup complete, and triggers a hinted scan per created library.
   */
  const finalizeImport = useCallback(async (): Promise<{
    ok: boolean;
    scanErrors: string[];
    error: string | null;
  }> => {
    setFinalizing(true);
    setFinalizeError(null);
    const scanErrors: string[] = [];
    const fail = (message: string) => {
      setFinalizing(false);
      setFinalizeError(message);
      return { ok: false, scanErrors, error: message };
    };
    const normPath = (p: string) =>
      p.trim().replace(/[\\/]+$/, "").toLowerCase();

    // Safety net: the backend requires a mapping for every source root it
    // warmed (configured root folders AND the folders titles actually live in).
    // If any detected root is still unmapped — e.g. the preview was reloaded
    // after a refresh and surfaced a content root that wasn't mapped — finalize
    // would fail server-side with a cryptic "missing mapping for source … root".
    // Catch it here with actionable guidance instead.
    const unmappedDetected = detectedRoots.filter((root) => !assign[root.id]);
    if (connectedArrSessionIds.length > 0 && unmappedDetected.length > 0) {
      return fail(
        `Some detected source folders aren't mapped to a library yet (e.g. "${unmappedDetected[0].arrRootPath}"). Go back to the Libraries step to map them.`,
      );
    }

    const { librariesToCreate } = buildMappings();

    // Cross-library guard: the backend rejects a root path already owned by
    // another library, so two drafts sharing an effective root path can't both
    // be created. Surface it up front instead of failing mid-create.
    const pathOwner = new Map<string, { id: string; name: string }>();
    for (const { draft, rootPaths } of librariesToCreate) {
      for (const path of rootPaths) {
        const owner = pathOwner.get(normPath(path));
        if (owner && owner.id !== draft.id) {
          return fail(
            `Root "${path}" is mapped to more than one library (${owner.name} and ${draft.name}). A root can belong to only one library.`,
          );
        }
        pathOwner.set(normPath(path), { id: draft.id, name: draft.name });
      }
    }

    // Resumable: reuse libraries resolved on a prior (failed) attempt so a retry
    // does only the pending work and never re-creates (and root-conflicts on)
    // libraries that already exist. Existing/default libraries are UPDATED with
    // their mapped roots; user-added libraries are CREATED.
    const createdByDraftId = createdLibrariesRef.current;
    for (const { draft, rootPaths } of librariesToCreate) {
      if (createdByDraftId.has(draft.id)) continue;
      const roots = rootPaths.map((path, index) => ({
        path,
        isDefault: index === 0,
      }));
      const settings = {
        qualityProfileId: draft.qualityProfileId,
        scoringPersona: draft.scoringPersona,
      };
      let resolvedId: string | null = null;
      // Default libraries: update in place if they exist. The default may not
      // exist yet during onboarding, so fall through to create on failure.
      if (draft.existingLibraryId) {
        const { data } = await client
          .mutation(updateLibraryMutation, {
            input: { libraryId: draft.existingLibraryId, roots, settings },
          })
          .toPromise();
        if (data?.updateLibrary?.id) {
          resolvedId = data.updateLibrary.id as string;
        }
      }
      if (!resolvedId) {
        const { data, error } = await client
          .mutation(createLibraryMutation, {
            input: { facet: draft.facet, name: draft.name, roots, settings },
          })
          .toPromise();
        const created = data?.createLibrary;
        if (error || !created?.id) {
          return fail(
            `${gqlError(error) || "Failed to create library"}: ${draft.name}`,
          );
        }
        resolvedId = created.id as string;
      }
      createdByDraftId.set(draft.id, resolvedId);
    }

    // Build mappings, deduped by the backend's mapping key so duplicate manual
    // roots (same library + path) don't trip "duplicate source root mapping".
    const mappings: ExternalImportSourceLibraryMappingInput[] = [];
    const seenMappingKey = new Set<string>();
    for (const root of roots) {
      const draftId = assign[root.id];
      if (!draftId) continue;
      const libraryId = createdByDraftId.get(draftId);
      const draft = libraries.find((lib) => lib.id === draftId);
      if (!libraryId || !draft) continue;
      const scryerRootPath = effectiveRootPath(root);
      const mappingKey = root.manual
        ? `manual|${libraryId}|${normPath(scryerRootPath)}`
        : `src|${root.sourceWarmupSessionId ?? ""}|${root.sourceKey ?? ""}|${normPath(root.arrRootPath)}`;
      if (seenMappingKey.has(mappingKey)) continue;
      seenMappingKey.add(mappingKey);
      mappings.push({
        sourceWarmupSessionId: root.sourceWarmupSessionId,
        sourceKey: root.sourceKey,
        kind: root.manual ? null : (root.kind as ExternalArrSourceKind),
        arrRootPath: root.arrRootPath,
        scryerRootPath,
        libraryId,
        facet: draft.facet,
      });
    }

    const { data: finalizeData, error: finalizeErr } = await client
      .mutation(finalizeExternalImportMutation, {
        input: {
          sourceWarmupSessionIds: connectedArrSessionIds,
          mappings,
        },
      })
      .toPromise();
    const finalized = finalizeData?.finalizeExternalImport;
    if (finalizeErr || !finalized?.monitorWarmupSessionId) {
      return fail(gqlError(finalizeErr) || "Failed to finalize import");
    }
    const monitorWarmupSessionId = finalized.monitorWarmupSessionId as string;

    const { data: completeData, error: completeErr } = await client
      .mutation(completeSetupMutation, {})
      .toPromise();
    if (completeErr || !completeData?.completeSetup?.completed) {
      return fail(gqlError(completeErr) || "Failed to complete setup");
    }

    // Scan each created library, passing the warmup session for import hints.
    const createdLibraryIds = Array.from(new Set(createdByDraftId.values()));
    for (const libraryId of createdLibraryIds) {
      const { error: scanErr } = await client
        .mutation(scanLibraryMutation, {
          input: { libraryId, importWarmupSessionId: monitorWarmupSessionId },
        })
        .toPromise();
      if (scanErr) scanErrors.push(gqlError(scanErr));
    }

    setFinalizing(false);
    // Setup is complete — drop both the local draft and the server secret draft.
    clearPersistedImportWizardState();
    void client
      .mutation(clearExternalImportSetupSecretDraftMutation, {})
      .toPromise();
    return { ok: true, scanErrors, error: null };
  }, [
    client,
    buildMappings,
    roots,
    detectedRoots,
    assign,
    libraries,
    connectedArrSessionIds,
  ]);

  // ── Derived summary counts ─────────────────────────────────────────────────
  const summary = useMemo(() => {
    const mappedRoots = roots.filter((root) => assign[root.id]);
    const remappedRoots = roots.filter((root) => isRootRemapped(root));
    const sonarrCount = arrInstances.filter(
      (inst) => inst.kind === "SONARR" && inst.status === "connected",
    ).length;
    const radarrCount = arrInstances.filter(
      (inst) => inst.kind === "RADARR" && inst.status === "connected",
    ).length;
    const selectedDcCount = preview
      ? preview.downloadClients.filter((dc) => selectedDcKeys.has(dc.dedupKey))
          .length
      : 0;
    const selectedIdxCount = preview
      ? preview.indexers.filter(
          (idx) => idx.supported && selectedIdxKeys.has(idx.dedupKey),
        ).length
      : 0;
    return {
      libraryCount: mappedLibraries.length,
      instancesConnected: sonarrCount + radarrCount,
      sonarrCount,
      radarrCount,
      rootsMapped: mappedRoots.length,
      pathsRemapped: remappedRoots.length,
      downloadClients: selectedDcCount,
      indexers: selectedIdxCount,
    };
  }, [
    roots,
    assign,
    arrInstances,
    mappedLibraries,
    preview,
    selectedDcKeys,
    selectedIdxKeys,
  ]);

  // The backend rejects finalize unless EVERY warmed (detected) root is mapped,
  // so the Libraries step can only advance once the tray has no detected roots
  // left and every assigned manual root has a non-empty path.
  const allDetectedRootsMapped = useMemo(
    () => detectedRoots.every((root) => Boolean(assign[root.id])),
    [detectedRoots, assign],
  );
  const hasBlankAssignedManualRoot = useMemo(
    () =>
      manualRoots.some(
        (root) => Boolean(assign[root.id]) && !effectiveRootPath(root).trim(),
      ),
    [manualRoots, assign],
  );
  const mappingReady =
    allDetectedRootsMapped &&
    !hasBlankAssignedManualRoot &&
    !assignedRootPathValidationLoading &&
    invalidAssignedRootIds.length === 0;

  // Settled = nothing left to wait on: no arr sessions, or every connected arr
  // source's warmup has reached a terminal state in the latest preview.
  const previewSettled = useMemo(() => {
    if (connectedArrSessionIds.length === 0) return true;
    const sources = preview?.arrSources ?? [];
    return (
      sources.length > 0 &&
      sources.every(
        (s) =>
          s.status === "COMPLETED" ||
          s.status === "FAILED" ||
          s.status === "CANCELED",
      )
    );
  }, [preview, connectedArrSessionIds]);

  // Every selected client/indexer that needs an operator-supplied secret has one.
  const sourcesReady = useMemo(() => {
    if (!prowlarrWarmupReady) return false;
    if (!preview) return true;
    const apiKeyClientTypes = new Set(["sabnzbd", "weaver"]);
    const dcOk = preview.downloadClients.every((dc) => {
      if (!selectedDcKeys.has(dc.dedupKey)) return true;
      const type = dc.scryerClientType?.trim().toLowerCase() ?? null;
      const needsApiKey =
        dc.supported &&
        !dc.apiKeyPresent &&
        type !== null &&
        apiKeyClientTypes.has(type);
      if (needsApiKey && !(dcApiKeyOverrides[dc.dedupKey] ?? "").trim()) {
        return false;
      }
      if (
        dc.supported &&
        dc.requiresPasswordOverride &&
        !(dcPasswordOverrides[dc.dedupKey] ?? "").trim()
      ) {
        return false;
      }
      return true;
    });
    if (!dcOk) return false;
    return preview.indexers.every((idx) => {
      if (!selectedIdxKeys.has(idx.dedupKey)) return true;
      if (
        idx.supported &&
        idx.requiresApiKeyOverride &&
        !(idxApiKeyOverrides[idx.dedupKey] ?? "").trim()
      ) {
        return false;
      }
      return true;
    });
  }, [
    preview,
    prowlarrWarmupReady,
    selectedDcKeys,
    selectedIdxKeys,
    dcApiKeyOverrides,
    dcPasswordOverrides,
    idxApiKeyOverrides,
  ]);

  // Persist NON-SENSITIVE user input so a refresh doesn't reset the wizard.
  // Instance API keys are stripped here (secrets live in the server draft);
  // server-derived state (preview, aggregate progress) is re-fetched.
  useEffect(() => {
    savePersistedImportWizardState({
      instances: instances.map((inst) => ({ ...inst, apiKey: "" })),
      manualRoots,
      remaps,
      assign,
      libraries,
      selectedDcKeys: [...selectedDcKeys],
      selectedIdxKeys: [...selectedIdxKeys],
      dcSelectionSeeded: dcSelectionSeeded.current,
      idxSelectionSeeded: idxSelectionSeeded.current,
      executeResult,
    });
  }, [
    instances,
    manualRoots,
    remaps,
    assign,
    libraries,
    selectedDcKeys,
    selectedIdxKeys,
    executeResult,
  ]);

  return {
    // connect
    instances,
    arrInstances,
    prowlarrInstance,
    instancesByKind,
    addInstance,
    removeInstance,
    setInstanceName,
    setInstanceConnectionField,
    verifyInstance,
    connectionReady,
    canLeaveConnect,
    connectedArrSessionIds,
    prowlarrWarmupSessionId,
    prowlarrWarmupProgress,
    prowlarrWarmupReady,
    retryProwlarrWarmup,
    // server secret draft
    secretDraftOwnedByOther,
    secretDraftOverwroteOther,
    // preview / board
    preview,
    previewing,
    previewError,
    loadPreview,
    roots,
    rootById,
    trayRoots,
    rootsForLibrary,
    assign,
    assignRoot,
    setRootRemap,
    addManualRoot,
    setManualRootPath,
    removeManualRoot,
    allDetectedRootsMapped,
    invalidAssignedRootIds,
    assignedRootPathValidationLoading,
    mappingReady,
    previewSettled,
    // libraries
    libraries,
    mappedLibraries,
    addLibrary,
    renameLibrary,
    removeLibrary,
    // quality
    qualityProfiles,
    qualityReady,
    setLibraryQualityProfile,
    setLibraryPersona,
    // sources
    selectedDcKeys,
    selectedIdxKeys,
    toggleDownloadClient,
    toggleIndexer,
    dcApiKeyOverrides,
    dcPasswordOverrides,
    idxApiKeyOverrides,
    setDownloadClientApiKeyOverride,
    setDownloadClientPasswordOverride,
    setIndexerApiKeyOverride,
    sourcesReady,
    executing,
    executeError,
    executeResult,
    executeSources,
    // summary / finalize
    aggregateProgress,
    pollAggregateProgress,
    warmupComplete,
    warmupSettled,
    warmupFailed,
    warmupErrorMessage,
    warmupSessionLost,
    retryWarmup,
    recoverLostWarmup,
    pendingReverify,
    clearPendingReverify,
    finalizing,
    finalizeError,
    finalizeImport,
    summary,
  };
}

function defaultLibraryName(
  facet: WizardFacet,
  existing: ImportLibraryDraft[],
): string {
  const base =
    facet === "MOVIE" ? "Movies" : facet === "SERIES" ? "Series" : "Anime";
  const sameFacet = existing.filter((lib) => lib.facet === facet).length;
  return sameFacet === 0 ? base : `${base} ${sameFacet + 1}`;
}

/** The three per-facet default libraries the board always starts with. */
function defaultLibraryDrafts(): ImportLibraryDraft[] {
  const facets: WizardFacet[] = ["MOVIE", "SERIES", "ANIME"];
  return facets.map((facet) => {
    // Server default-library ids are lowercase (`movie_default_library`);
    // WizardFacet is SCREAMING_SNAKE.
    const id = `${facet.toLowerCase()}_default_library`;
    return {
      id,
      facet,
      name: facet === "MOVIE" ? "Movies" : facet === "SERIES" ? "Series" : "Anime",
      qualityProfileId: null,
      scoringPersona: "BALANCED",
      existingLibraryId: id,
      isDefault: true,
    };
  });
}

export type UseExternalImportSetupReturn = ReturnType<
  typeof useExternalImportSetup
>;
