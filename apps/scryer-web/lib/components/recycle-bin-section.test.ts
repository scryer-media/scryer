import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import ts from "typescript";
import type { SettingsRecycleBinSection } from "../../components/views/settings/settings-recycle-bin-section.tsx";

const require = createRequire(import.meta.url);
type Props = Parameters<typeof SettingsRecycleBinSection>[0];
type Element = { type: unknown; props: Record<string, unknown> };

function elements(node: unknown): Element[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!node || typeof node !== "object" || !("props" in node)) return [];
  const element = node as Element;
  return [element, ...elements(element.props.children)];
}

function mountSection() {
  const slots: unknown[] = [];
  let cursor = 0;
  const overrides: Record<string, unknown> = {
    react: {
      Fragment: require("react").Fragment,
      useMemo: (factory: () => unknown) => factory(),
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
  new Function("require", "module", "exports", code)(
    (id: string) => {
      if (overrides[id]) return overrides[id];
      if (id.startsWith("@/components/") || id === "lucide-react")
        return new Proxy({}, { get: (_target, name) => String(name) });
      return require(id);
    },
    module,
    module.exports,
  );
  const items = ["first", "second"].map((id) => ({
    id,
    titleId: "title",
    titleName: "Fixture title",
    libraryId: "library",
    libraryName: "Fixture library",
    fileName: `${id}.mkv`,
    originalPath: `/media/${id}.mkv`,
    mediaRoot: "/media",
    sizeBytes: 128,
    recycledAt: "2026-01-01T00:00:00Z",
    scheduledDeletionAt: "2026-01-08T00:00:00Z",
    reason: "upgrade_replaced",
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
  };
  return () => {
    cursor = 0;
    return elements(module.exports.SettingsRecycleBinSection(props));
  };
}

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
