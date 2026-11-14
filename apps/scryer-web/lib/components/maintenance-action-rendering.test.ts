import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType, type Context } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { Translate } from "@/components/root/types";
import type { MaintenanceCandidatesPanelProps } from "@/components/views/settings/settings-maintenance-operations-panel";
import type {
  MaintenanceCandidate,
  MaintenanceDeclaredActions,
} from "../types/maintenance-rule-sets.ts";
import type { RuleValidationDiagnostic } from "../utils/rego-diagnostics.ts";

type Views = {
  CandidatesPanel: ComponentType<MaintenanceCandidatesPanelProps>;
  DeclaredSummary: ComponentType<{
    declared: MaintenanceDeclaredActions;
    idPrefix: string;
  }>;
  SourceFindings: ComponentType<{
    source: string;
    declared: MaintenanceDeclaredActions | null | undefined;
    warnings: readonly RuleValidationDiagnostic[];
    checked: boolean;
  }>;
  render: (element: ReturnType<typeof createElement>) => string;
};

async function withMaintenanceViews(run: (views: Views) => void) {
  const server = await createServer({
    root: fileURLToPath(new URL("../..", import.meta.url)),
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
    plugins: [
      {
        name: "maintenance-date-format-fixture",
        enforce: "pre",
        load(id) {
          if (id.endsWith("/lib/context/ui-settings-context.tsx")) {
            return 'export function useUiDateTimeFormat() { return "LOCALE"; }';
          }
        },
      },
    ],
  });
  try {
    const [panel, summary, translation] = await Promise.all([
      server.ssrLoadModule(
        "/components/views/settings/settings-maintenance-operations-panel.tsx",
      ),
      server.ssrLoadModule(
        "/components/views/settings/maintenance-action-summary.tsx",
      ),
      server.ssrLoadModule("/lib/context/translate-context.tsx"),
    ]);
    const TranslateContext =
      translation.TranslateContext as Context<Translate | null>;
    run({
      CandidatesPanel: panel.MaintenanceCandidatesPanel,
      DeclaredSummary: summary.MaintenanceDeclaredActionsSummary,
      SourceFindings: summary.MaintenanceRuleSourceFindings,
      render: (element) =>
        renderToStaticMarkup(
          createElement(
            TranslateContext.Provider,
            { value: (key) => key },
            element,
          ),
        ),
    });
  } finally {
    await server.close();
  }
}

function declared(
  overrides: Partial<MaintenanceDeclaredActions> = {},
): MaintenanceDeclaredActions {
  return {
    heads: [],
    destructive: false,
    qualityProfiles: [],
    addTags: [],
    removeTags: [],
    libraries: [],
    roots: [],
    notifications: [],
    ...overrides,
  };
}

function candidate(
  overrides: Partial<MaintenanceCandidate>,
): MaintenanceCandidate {
  return {
    fileCount: 1,
    totalSizeBytes: 1024,
    storageRootFileCount: null,
    storageRootTotalSizeBytes: null,
    subjectLabel: "Synthetic Movie (2001)",
    subjectKind: "TITLE",
    subjectId: "title-1",
    id: "candidate-1",
    ruleSetId: "rule-1",
    ruleName: "synthetic_rule",
    revisionNumber: 1,
    titleId: "title-1",
    titleName: "Synthetic Movie",
    libraryId: "lib-1",
    facet: "movie",
    state: "PENDING_ACTION",
    stateReason: "",
    reasonCodes: [],
    actions: null,
    actionKind: "DO_NOTHING",
    actionSequence: null,
    sequenceSteps: [],
    graceDays: 0,
    matchGeneration: 1,
    firstMatchedAt: "2026-01-01T00:00:00Z",
    lastMatchedAt: "2026-01-01T00:00:00Z",
    dueAt: "2026-01-02T00:00:00Z",
    heldSince: null,
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function panelProps(
  candidates: MaintenanceCandidate[],
): MaintenanceCandidatesPanelProps {
  return {
    candidates,
    candidatesLoading: false,
    candidatesError: null,
    ruleSetRecords: [],
    libraries: [{ id: "lib-1", name: "Movies" }],
    gates: null,
    candidateRuleFilter: "all",
    setCandidateRuleFilter: () => {},
    candidateStateFilter: "all",
    setCandidateStateFilter: () => {},
    candidateLibraryFilter: "all",
    setCandidateLibraryFilter: () => {},
    refreshCandidates: () => {},
    excludeCandidate: () => {},
  };
}

test("a candidate shows the actions its rule decided, and a historical one its recorded kind", async () => {
  await withMaintenanceViews(({ CandidatesPanel, render }) => {
    const markup = render(
      createElement(
        CandidatesPanel,
        panelProps([
          candidate({
            id: "decided",
            actions: {
              heads: ["SET_QUALITY_PROFILE", "SEARCH", "NOTIFY"],
              qualityProfile: "Compact 1080p",
              addTags: [],
              removeTags: [],
              library: null,
              root: null,
              notification: "Moved to a smaller profile",
            },
          }),
          candidate({
            id: "historical",
            actions: null,
            actionKind: "UNMONITOR_SCOPE_KEEP_FILES",
          }),
        ]),
      ),
    );

    assert.match(
      markup,
      /id="settings-maintenance-candidate-action-decided-set-quality-profile"[^>]*>.*?Compact 1080p/,
    );
    assert.match(
      markup,
      /id="settings-maintenance-candidate-action-decided-search"/,
    );
    assert.match(markup, /Moved to a smaller profile/);
    assert.doesNotMatch(
      markup,
      /id="settings-maintenance-candidate-action-decided-delete/,
    );
    // The historical row renders from its recorded kind, not from actions.
    assert.doesNotMatch(
      markup,
      /id="settings-maintenance-candidate-action-historical/,
    );
    assert.match(markup, /settings\.maintenanceActionUnmonitorScopeKeepFiles/);
  });
});

test("the destructive marker follows declaredActions.destructive", async () => {
  await withMaintenanceViews(({ DeclaredSummary, render }) => {
    const destructive = render(
      createElement(DeclaredSummary, {
        declared: declared({
          heads: ["DELETE_TITLE", "NOTIFY"],
          destructive: true,
          notifications: ["Removed"],
        }),
        idPrefix: "rule",
      }),
    );
    assert.match(destructive, /id="rule-destructive"/);
    assert.match(
      destructive,
      /id="rule-delete-title"[^>]*data-maintenance-action-head="DELETE_TITLE"/,
    );

    const reversible = render(
      createElement(DeclaredSummary, {
        declared: declared({
          heads: ["MOVE_TO_ROOT", "FETCH_SUBTITLES"],
          roots: ["/media/archive"],
        }),
        idPrefix: "rule",
      }),
    );
    assert.doesNotMatch(reversible, /id="rule-destructive"/);
    assert.match(reversible, /\/media\/archive/);
    // Execution order: subtitles are fetched before the move.
    assert.ok(
      reversible.indexOf('id="rule-fetch-subtitles"') <
        reversible.indexOf('id="rule-move-to-root"'),
    );
  });
});

test("warnings render apart from errors with their line, and an unparseable source names no actions", async () => {
  await withMaintenanceViews(({ SourceFindings, render }) => {
    const source = "helper if true\n\nunmonitor if true\n";
    const markup = render(
      createElement(SourceFindings, {
        source,
        declared: declared({ heads: ["UNMONITOR"] }),
        warnings: [
          {
            code: "RULE_UNREFERENCED_RULE",
            message: "server words",
            // The server counts the package and import lines it inserts.
            line: 3,
            column: 1,
          },
        ],
        checked: true,
      }),
    );
    assert.match(
      markup,
      /id="settings-maintenance-rule-warning-rule-unreferenced-rule-0"[^>]*data-diagnostic-code="RULE_UNREFERENCED_RULE"/,
    );
    assert.match(markup, /error\.ruleUnreferencedRule/);
    assert.doesNotMatch(markup, /server words/);
    assert.match(
      markup,
      /id="settings-maintenance-rule-declared-action-unmonitor"/,
    );

    const unparseable = render(
      createElement(SourceFindings, {
        source,
        declared: null,
        warnings: [],
        checked: true,
      }),
    );
    assert.match(
      unparseable,
      /id="settings-maintenance-rule-declared-actions-unavailable"/,
    );
    assert.doesNotMatch(unparseable, /id="settings-maintenance-rule-warnings"/);

    assert.equal(
      render(
        createElement(SourceFindings, {
          source,
          declared: undefined,
          warnings: [],
          checked: false,
        }),
      ),
      "",
    );
  });
});
