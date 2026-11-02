import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import ts from "typescript";
import type { SettingsRecycleBinSection } from "../../components/views/settings/settings-recycle-bin-section.tsx";

const require = createRequire(import.meta.url);
type Props = Parameters<typeof SettingsRecycleBinSection>[0];
type Element = { type: unknown; props: Record<string, unknown> };

class ClickTarget {
  readonly selector: string;

  constructor(selector: string) {
    this.selector = selector;
  }

  closest(selectors: string) {
    return selectors.split(", ").includes(this.selector) ? this : null;
  }
}

function elements(node: unknown): Element[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!node || typeof node !== "object" || !("props" in node)) return [];
  const element = node as Element;
  return [element, ...elements(element.props.children)];
}

function mountSection(
  itemOverrides: Partial<Props["items"][number]> = {},
  initialProps: Partial<Props> = {},
) {
  const slots: unknown[] = [];
  const effects: Array<() => void> = [];
  const scrolls: string[] = [];
  const focused: string[] = [];
  let tree: Element[] = [];
  let cursor = 0;
  const overrides: Record<string, unknown> = {
    "react-router": { Link: "Link" },
    react: {
      Fragment: require("react").Fragment,
      useMemo: (factory: () => unknown) => factory(),
      useRef(initial: unknown) {
        const index = cursor++;
        if (!(index in slots)) slots[index] = { current: initial };
        return slots[index];
      },
      useEffect(effect: () => void, dependencies: unknown[]) {
        const index = cursor++;
        const previous = slots[index] as unknown[] | undefined;
        if (
          !previous ||
          dependencies.some((value, i) => !Object.is(value, previous[i]))
        )
          effects.push(effect);
        slots[index] = dependencies;
      },
      useState(initial: unknown) {
        const index = cursor++;
        if (!(index in slots)) slots[index] = initial;
        return [
          slots[index],
          (value: unknown) => {
            slots[index] =
              typeof value === "function" ? value(slots[index]) : value;
          },
        ];
      },
    },
    "@/lib/context/translate-context": {
      useTranslate: () => (key: string) => key,
    },
    "@/lib/context/ui-settings-context": { useUiDateTimeFormat: () => "auto" },
    "@/lib/utils/recycle-bin": require("../utils/recycle-bin.ts"),
    "@/lib/utils/dom-ids": require("../utils/dom-ids.ts"),
    "@/lib/utils/calendar-event-href": require("../utils/calendar-event-href.ts"),
    "@/lib/utils/date-format": { formatUiDateTime: (value: string) => value },
    "./recycle-bin-history": { RecycleBinHistory: "recycle-history" },
  };
  const source = readFileSync(
    new URL(
      "../../components/views/settings/settings-recycle-bin-section.tsx",
      import.meta.url,
    ),
    "utf8",
  );
  const code = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      jsx: ts.JsxEmit.ReactJSX,
    },
  }).outputText;
  const module = {
    exports: {} as { SettingsRecycleBinSection: (props: Props) => unknown },
  };
  new Function("require", "module", "exports", "Element", "document", code)(
    (id: string) => {
      if (overrides[id]) return overrides[id];
      if (id.startsWith("@/components/") || id === "lucide-react")
        return new Proxy({}, { get: (_target, name) => String(name) });
      return require(id);
    },
    module,
    module.exports,
    ClickTarget,
    {
      getElementById(id: string) {
        if (!tree.some((node) => node.props.id === id)) return null;
        return {
          scrollIntoView: () => scrolls.push(id),
          focus: () => focused.push(id),
        };
      },
    },
  );
  const items = ["first", "second"].map((id) => ({
    id,
    titleId: "title",
    titleName: "Fixture title",
    titleFacet: "movie",
    episodeId: null,
    libraryId: "library",
    libraryName: "Fixture library",
    fileName: `${id}.mkv`,
    originalPath: `/media/${id}.mkv`,
    mediaRoot: "/media",
    sizeBytes: 128,
    recycledAt: "2026-01-01T00:00:00Z",
    scheduledDeletionAt: "2026-01-08T00:00:00Z",
    reason: "upgrade_replaced",
    ...itemOverrides,
  }));
  const noop = () => {};
  const props: Props = {
    enabled: true,
    path: null,
    retentionDays: 7,
    effectivePaths: [],
    validationError: null,
    relocation: null,
    settingsLoading: false,
    settingsSaving: false,
    canManageConfig: true,
    canManageItems: true,
    libraries: [],
    librariesLoading: false,
    selectedLibraryIds: [],
    items,
    totalCount: 2,
    loading: false,
    mutatingId: null,
    pendingItemIds: new Set(),
    selectedItemIds: new Set(),
    onEnabledChange: noop,
    onLocationSave: noop,
    onSelectedLibraryIdsChange: noop,
    onSelectedItemIdsChange: noop,
    onRestoreItems: noop,
    onDeleteItems: noop,
    onEmptyAll: noop,
    ...initialProps,
  };
  return Object.assign(
    (nextProps: Partial<Props> = {}) => {
      cursor = 0;
      effects.length = 0;
      tree = elements(
        module.exports.SettingsRecycleBinSection({ ...props, ...nextProps }),
      );
      for (const effect of effects) effect();
      return tree;
    },
    { scrolls, focused },
  );
}

test("a dashboard target opens its history and scrolls once after async rows load", () => {
  const render = mountSection({}, { initialHistoryItemId: "second" });
  const histories = (nodes: Element[]) =>
    nodes
      .filter((node) => node.type === "recycle-history")
      .map((node) => node.props.id);
  assert.deepEqual(histories(render({ loading: true, items: [] })), []);
  assert.deepEqual(render.scrolls, []);
  render({ librariesLoading: true, items: [] });
  assert.deepEqual(render.scrolls, []);
  let tree = render();
  assert.deepEqual(histories(tree), ["second"]);
  assert.deepEqual(render.scrolls, ["settings-recycle-bin-row-second"]);
  assert.deepEqual(render.focused, ["settings-recycle-bin-history-second"]);

  const toggle = tree.find(
    (node) => node.props.id === "settings-recycle-bin-history-second",
  );
  assert.ok(toggle);
  (toggle.props.onClick as () => void)();
  assert.deepEqual(histories(render()), []);
  render({ loading: true });
  tree = render();
  assert.deepEqual(histories(tree), []);
  assert.deepEqual(render.scrolls, ["settings-recycle-bin-row-second"]);
});

test("an unavailable dashboard target never opens or scrolls to a different item", () => {
  const render = mountSection({}, { initialHistoryItemId: "missing" });
  assert.equal(
    render().some((node) => node.type === "recycle-history"),
    false,
  );
  assert.deepEqual(render.scrolls, []);
  assert.deepEqual(render.focused, []);
});

test("history mounts only for the expanded item and changes scope when another row opens", () => {
  const render = mountSection();
  const history = (tree: Element[]) =>
    tree.filter((node) => node.type === "recycle-history");
  const toggle = (tree: Element[], id: string) => {
    const button = tree.find(
      (node) => node.props.id === `settings-recycle-bin-history-${id}`,
    );
    assert.ok(button);
    (button.props.onClick as () => void)();
  };
  let tree = render();
  assert.equal(history(tree).length, 0);
  toggle(tree, "first");
  tree = render();
  assert.deepEqual(
    history(tree).map((node) => node.props.id),
    ["first"],
  );
  toggle(tree, "second");
  tree = render();
  assert.deepEqual(
    history(tree).map((node) => node.props.id),
    ["second"],
  );
  toggle(tree, "second");
  assert.equal(history(render()).length, 0);
});

test("file titles link to movies, exact episodes, or a title overview when no single episode is known", () => {
  for (const [item, expected] of [
    [{ titleFacet: "movie", episodeId: null }, "/movies?id=title"],
    [
      { titleFacet: "series", episodeId: "episode-1" },
      "/series?id=title&episodeId=episode-1",
    ],
    [
      { titleFacet: "anime", episodeId: "episode-2" },
      "/anime?id=title&episodeId=episode-2",
    ],
    [{ titleFacet: "series", episodeId: null }, "/series?id=title"],
  ] as const) {
    const tree = mountSection(item)();
    const link = tree.find(
      (node) => node.props.id === "settings-recycle-bin-item-link-first",
    );
    assert.ok(link);
    assert.equal(link.type, "Link");
    assert.equal(link.props.to, expected);
  }
  for (const item of [{ titleId: null }, { titleFacet: null }]) {
    assert.equal(
      mountSection(item)().some((node) => node.type === "Link"),
      false,
    );
  }
});

test("row clicks toggle history while embedded controls leave disclosure state unchanged", () => {
  const render = mountSection();
  const clickRow = (selector: string) => {
    const row = render().find(
      (node) => node.props.id === "settings-recycle-bin-row-first",
    );
    assert.ok(row);
    (row.props.onClick as (event: { target: ClickTarget }) => void)({
      target: new ClickTarget(selector),
    });
  };
  const expanded = () =>
    render().some((node) => node.type === "recycle-history");
  const controls = [
    "button",
    "a",
    "input",
    "select",
    "textarea",
    "[role='button']",
    "[role='checkbox']",
  ];

  for (const control of controls) {
    clickRow(control);
    assert.equal(expanded(), false, control);
  }
  clickRow("span");
  assert.equal(expanded(), true);
  for (const control of controls) {
    clickRow(control);
    assert.equal(expanded(), true, control);
  }
  clickRow("span");
  assert.equal(expanded(), false);
});
