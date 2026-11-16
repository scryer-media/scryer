import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType, type Context } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Provider, type Client } from "urql";
import { createServer } from "vite";
import type { Translate } from "@/components/root/types";
import type { LifecycleCandidatesPanelProps } from "@/components/views/settings/settings-lifecycle-operations-panel";
import type {
  LifecycleCandidate,
  LifecycleDeclaredActions,
} from "../types/lifecycle-rule-sets.ts";
import type { RuleValidationDiagnostic } from "../utils/rego-diagnostics.ts";
import type {
  RequestDeclaredActions,
  RequestShaping,
} from "../types/request-rule-sets.ts";
import {
  LIFECYCLE_SERVED_PAYLOAD,
  REQUEST_SERVED_PAYLOAD,
} from "../utils/rule-builder-contract-fixture.ts";

type Views = {
  CandidatesPanel: ComponentType<LifecycleCandidatesPanelProps>;
  DeclaredSummary: ComponentType<{
    declared: LifecycleDeclaredActions;
    idPrefix: string;
  }>;
  SourceFindings: ComponentType<{
    source: string;
    declared: LifecycleDeclaredActions | null | undefined;
    warnings: readonly RuleValidationDiagnostic[];
    checked: boolean;
  }>;
  RequestDeclared: ComponentType<{
    declared: RequestDeclaredActions;
    idPrefix: string;
  }>;
  RequestShaping: ComponentType<{
    shaping: RequestShaping | null | undefined;
    idPrefix: string;
  }>;
  render: (element: ReturnType<typeof createElement>) => string;
};

async function withLifecycleViews(run: (views: Views) => void) {
  const server = await createServer({
    root: fileURLToPath(new URL("../..", import.meta.url)),
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
    plugins: [
      {
        name: "lifecycle-date-format-fixture",
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
    const [panel, summary, translation, served] = await Promise.all([
      server.ssrLoadModule(
        "/components/views/settings/settings-lifecycle-operations-panel.tsx",
      ),
      server.ssrLoadModule(
        "/components/views/settings/lifecycle-action-summary.tsx",
      ),
      server.ssrLoadModule("/lib/context/translate-context.tsx"),
      server.ssrLoadModule("/lib/utils/rule-builder-served.ts"),
    ]);
    // The served descriptors label and order the heads, as the page's one
    // read per family would.
    await served.loadServedRuleBuilder(
      "lifecycle",
      async () => LIFECYCLE_SERVED_PAYLOAD,
    );
    await served.loadServedRuleBuilder("request", async () => REQUEST_SERVED_PAYLOAD);
    const TranslateContext =
      translation.TranslateContext as Context<Translate | null>;
    run({
      CandidatesPanel: panel.LifecycleCandidatesPanel,
      DeclaredSummary: summary.LifecycleDeclaredActionsSummary,
      SourceFindings: summary.LifecycleRuleSourceFindings,
      RequestDeclared: summary.RequestDeclaredActionsSummary,
      RequestShaping: summary.RequestShapingSummary,
      render: (element) =>
        renderToStaticMarkup(
          createElement(
            Provider,
            { value: {} as Client },
            createElement(
              TranslateContext.Provider,
              { value: (key) => key },
              element,
            ),
          ),
        ),
    });
  } finally {
    await server.close();
  }
}

function declared(
  overrides: Partial<LifecycleDeclaredActions> = {},
): LifecycleDeclaredActions {
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
  overrides: Partial<LifecycleCandidate>,
): LifecycleCandidate {
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
  candidates: LifecycleCandidate[],
): LifecycleCandidatesPanelProps {
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

test("a candidate shows the actions its rule decided", async () => {
  await withLifecycleViews(({ CandidatesPanel, render }) => {
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
        ]),
      ),
    );

    assert.match(
      markup,
      /id="settings-lifecycle-candidate-action-decided-set-quality-profile"[^>]*>.*?Compact 1080p/,
    );
    assert.match(
      markup,
      /id="settings-lifecycle-candidate-action-decided-search"/,
    );
    assert.match(markup, /Moved to a smaller profile/);
    assert.doesNotMatch(
      markup,
      /id="settings-lifecycle-candidate-action-decided-delete/,
    );
  });
});

test("the destructive marker follows declaredActions.destructive", async () => {
  await withLifecycleViews(({ DeclaredSummary, render }) => {
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
      /id="rule-delete-title"[^>]*data-lifecycle-action-head="DELETE_TITLE"/,
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

    // A head the served descriptors do not list renders by its id, last.
    const unseen = render(
      createElement(DeclaredSummary, {
        declared: declared({ heads: ["ARCHIVE", "UNMONITOR"] }),
        idPrefix: "rule",
      }),
    );
    assert.match(
      unseen,
      /id="rule-archive"[^>]*data-lifecycle-action-head="ARCHIVE"[^>]*>(<[^>]*>)*ARCHIVE</,
    );
    // This render translates nothing, so a served head shows by its Rego name.
    assert.match(unseen, /id="rule-unmonitor"[^>]*>(<[^>]*>)*unmonitor</);
    assert.ok(unseen.indexOf('id="rule-unmonitor"') < unseen.indexOf('id="rule-archive"'));
  });
});

test("warnings render apart from errors with their line, and an unparseable source names no actions", async () => {
  await withLifecycleViews(({ SourceFindings, render }) => {
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
      /id="settings-lifecycle-rule-warning-rule-unreferenced-rule-0"[^>]*data-diagnostic-code="RULE_UNREFERENCED_RULE"/,
    );
    assert.match(markup, /error\.ruleUnreferencedRule/);
    assert.doesNotMatch(markup, /server words/);
    assert.match(
      markup,
      /id="settings-lifecycle-rule-declared-action-unmonitor"/,
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
      /id="settings-lifecycle-rule-declared-actions-unavailable"/,
    );
    assert.doesNotMatch(unparseable, /id="settings-lifecycle-rule-warnings"/);

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

test("a request rule's declared heads render in served order with their values", async () => {
  await withLifecycleViews(({ RequestDeclared, render }) => {
    const markup = render(
      createElement(RequestDeclared, {
        declared: {
          heads: ["ADD_TAGS", "SET_LEASE_DAYS", "APPROVE"],
          qualityProfiles: [],
          leaseDays: [0],
          monitors: [],
          requiredAudioLanguages: [],
          preferredAudioLanguages: [],
          addTags: ["sample-tag"],
          notifications: [],
        },
        idPrefix: "request-rule",
      }),
    );
    assert.match(markup, /id="request-rule-approve"[^>]*data-request-action-head="APPROVE"/);
    assert.match(markup, /requests\.leaseForever/);
    assert.match(markup, /sample-tag/);
    assert.ok(
      markup.indexOf('id="request-rule-approve"') <
        markup.indexOf('id="request-rule-set-lease-days"'),
    );
    assert.ok(
      markup.indexOf('id="request-rule-set-lease-days"') <
        markup.indexOf('id="request-rule-add-tags"'),
    );
  });
});

test("shaping renders each value with its rule and each conflict once, and nothing when empty", async () => {
  await withLifecycleViews(({ RequestShaping, render }) => {
    const markup = render(
      createElement(RequestShaping, {
        shaping: {
          qualityProfile: { value: "Sample Profile", ruleSetName: "Sample rule" },
          leaseDays: null,
          monitor: null,
          requiredAudioLanguages: null,
          preferredAudioLanguages: null,
          notifications: [],
          conflicts: ["SET_MONITOR", "SET_MONITOR"],
        },
        idPrefix: "decision-shaping",
      }),
    );
    assert.match(
      markup,
      /id="decision-shaping-set-quality-profile-0"[^>]*data-request-shaping-head="SET_QUALITY_PROFILE"/,
    );
    assert.match(markup, /Sample Profile/);
    assert.match(markup, /requests\.shapingSetBy/);
    assert.equal(markup.match(/data-request-shaping-conflict="SET_MONITOR"/g)?.length, 1);
    assert.match(markup, /id="decision-shaping-conflict-set-monitor"/);

    assert.equal(
      render(createElement(RequestShaping, { shaping: null, idPrefix: "decision-shaping" })),
      "",
    );
  });
});
