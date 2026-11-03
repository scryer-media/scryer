import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import ts from "typescript";

type Element = {
  type: string;
  props: {
    children?: unknown;
    open?: boolean;
    role?: string;
    onClick: () => void;
    onOpenChange: (open: boolean) => void;
  };
};
type Variables = {
  id: string;
  choice: string;
  confirmed: { itemId: string }[];
};
const require = createRequire(import.meta.url);
const conflict = (id: string) => ({
  downloadClientId: "primary",
  downloadClientType: "nzbget",
  downloadClientItemId: id,
  sourceTitle: `Release ${id}`,
  titleName: "Fixture",
  state: "DOWNLOADING",
});

function mount(responses: unknown[], refreshFails = false) {
  const slots: unknown[] = [];
  const initials: unknown[] = [];
  let cursor = 0;
  let complete: (() => void) | undefined;
  let refreshed = 0;
  const calls: Variables[] = [];
  const statuses: string[] = [];
  const react = {
    useRef(initial: unknown) {
      const index = cursor++;
      return (slots[index] ??= { current: initial });
    },
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in slots)) {
        slots[index] = initial;
        initials[index] = initial;
      }
      return [
        slots[index],
        (value: unknown) => {
          slots[index] = value;
          // The busy flag is the only boolean state; its return to false
          // ends a request, whatever order the hooks are declared in.
          if (initials[index] === false && value === false) complete?.();
        },
      ];
    },
  };
  const overrides: Record<string, unknown> = {
    react,
    "react/jsx-runtime": {
      jsx: (type: string, props: object) => ({ type, props }),
      jsxs: (type: string, props: object) => ({ type, props }),
      Fragment: "fragment",
    },
    urql: {
      useClient: () => ({
        mutation: (_query: unknown, variables: Variables) => {
          calls.push(variables);
          return { toPromise: async () => responses.shift() };
        },
      }),
    },
    "lucide-react": { Download: "icon" },
    "@/components/ui/button": { Button: "button" },
    "@/components/ui/dialog": {
      Dialog: "dialog",
      DialogContent: "content",
      DialogTitle: "title",
      DialogDescription: "description",
    },
    "@/lib/context/translate-context": {
      useTranslate: () => (key: string, values?: Record<string, unknown>) =>
        values?.count === undefined ? key : `${key}:${values.count}`,
    },
    "@/lib/context/global-status-context": {
      useGlobalStatus: () => (message: string) => statuses.push(message),
    },
    "@/lib/graphql/mutations": { queueStandbyReleaseMutation: "queue" },
    "@/lib/graphql/error-message": {
      userFacingGraphQlErrorMessage: () => "failure",
    },
  };
  const code = ts.transpileModule(
    readFileSync(
      new URL(
        "../../components/common/standby-download-button.tsx",
        import.meta.url,
      ),
      "utf8",
    ),
    {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        jsx: ts.JsxEmit.ReactJSX,
      },
    },
  ).outputText;
  const module = {
    exports: {} as { StandbyDownloadButton: (props: object) => Element },
  };
  new Function("require", "module", "exports", code)(
    (id: string) => overrides[id] ?? require(id),
    module,
    module.exports,
  );
  const render = () => {
    cursor = 0;
    return module.exports.StandbyDownloadButton({
      release: { id: "saved", releaseTitle: "Saved fixture" },
      onQueued: async () => {
        refreshed += 1;
        if (refreshFails) throw new Error("synthetic refresh failure");
      },
    });
  };
  const elements = (root: Element): Element[] => [
    root,
    ...[root.props.children]
      .flat()
      .filter((child): child is Element => !!child && typeof child === "object")
      .flatMap(elements),
  ];
  const click = async (label: string) => {
    const button = elements(render()).find(
      (item) =>
        item.type === "button" && [item.props.children].flat().includes(label),
    );
    assert.ok(button, label);
    const done = new Promise<void>((resolve) => {
      complete = resolve;
    });
    button.props.onClick();
    await done;
  };
  return {
    calls,
    statuses,
    render,
    elements,
    click,
    refreshed: () => refreshed,
  };
}

test(
  "standby direct queue refreshes only after acceptance",
  { timeout: 5000 },
  async () => {
    const h = mount([
      { data: { queueStandbyRelease: { grabbed: true, conflicts: [] } } },
    ]);
    await h.click("wanted.downloadStandby");
    assert.deepEqual(h.calls, [
      { id: "saved", choice: "CHECK", confirmed: [] },
    ]);
    assert.equal(h.refreshed(), 1);
  },
);

test(
  "accepted queue is not reported as submission failure when refresh fails",
  { timeout: 5000 },
  async () => {
    const h = mount(
      [{ data: { queueStandbyRelease: { grabbed: true, conflicts: [] } } }],
      true,
    );
    await h.click("wanted.downloadStandby");
    assert.equal(h.calls.length, 1);
    assert.equal(h.statuses.at(-1), "wanted.standbyQueuedRefreshFailed");
    assert.ok(
      !h.elements(h.render()).some((item) => item.props.role === "alert"),
    );
  },
);

for (const [label, choice] of [
  ["wanted.standbyQueue", "QUEUE"],
  ["wanted.standbyCancelAndQueue", "CANCEL_AND_QUEUE"],
]) {
  test(
    `standby ${choice} confirms the displayed set and refreshes`,
    { timeout: 5000 },
    async () => {
      const h = mount([
        {
          data: {
            queueStandbyRelease: {
              grabbed: false,
              conflicts: [conflict("one"), conflict("two")],
            },
          },
        },
        { data: { queueStandbyRelease: { grabbed: true, conflicts: [] } } },
      ]);
      await h.click("wanted.downloadStandby");
      assert.equal(h.refreshed(), 0);
      assert.equal(
        h.elements(h.render()).filter((item) => item.type === "li").length,
        2,
      );
      await h.click(label);
      assert.equal(h.calls[1].choice, choice);
      assert.deepEqual(
        h.calls[1].confirmed.map((item) => item.itemId),
        ["one", "two"],
      );
      assert.equal(h.refreshed(), 1);
      assert.equal(
        h.elements(h.render()).find((item) => item.type === "dialog")?.props
          .open,
        false,
      );
    },
  );
}

test(
  "standby changed conflicts replace the confirmation set; dismissal has no mutation",
  { timeout: 5000 },
  async () => {
    const h = mount([
      {
        data: {
          queueStandbyRelease: { grabbed: false, conflicts: [conflict("one")] },
        },
      },
      {
        data: {
          queueStandbyRelease: {
            grabbed: false,
            conflicts: [conflict("one"), conflict("new")],
          },
        },
      },
    ]);
    await h.click("wanted.downloadStandby");
    await h.click("wanted.standbyCancelAndQueue");
    const tree = h.render();
    assert.equal(
      h.elements(tree).filter((item) => item.type === "li").length,
      2,
    );
    h.elements(tree)
      .find((item) => item.type === "dialog")!
      .props.onOpenChange(false);
    assert.equal(h.calls.length, 2);
    assert.equal(h.refreshed(), 0);
    assert.equal(
      h.elements(h.render()).find((item) => item.type === "dialog")?.props.open,
      false,
    );
  },
);

test(
  "standby failure retains the confirmation for retry without refreshing",
  { timeout: 5000 },
  async () => {
    const h = mount([
      {
        data: {
          queueStandbyRelease: { grabbed: false, conflicts: [conflict("one")] },
        },
      },
      { error: new Error("synthetic cancellation failure") },
    ]);
    await h.click("wanted.downloadStandby");
    await h.click("wanted.standbyCancelAndQueue");
    assert.equal(h.refreshed(), 0);
    assert.equal(
      h.elements(h.render()).find((item) => item.type === "dialog")?.props.open,
      true,
    );
    assert.ok(
      h.elements(h.render()).some((item) => item.props.role === "alert"),
    );
  },
);

test(
  "cancel and queue reports conflicts that could not be cancelled as queued",
  { timeout: 5000 },
  async () => {
    const h = mount([
      {
        data: {
          queueStandbyRelease: {
            grabbed: false,
            conflicts: [conflict("one"), conflict("two")],
          },
        },
      },
      {
        data: {
          queueStandbyRelease: {
            grabbed: true,
            conflicts: [],
            cancelled: [conflict("one")],
            cancelFailed: [conflict("two")],
          },
        },
      },
    ]);
    await h.click("wanted.downloadStandby");
    await h.click("wanted.standbyCancelAndQueue");
    const tree = h.render();
    assert.ok(!h.elements(tree).some((item) => item.type === "li"));
    assert.ok(!h.elements(tree).some((item) => item.props.role === "alert"));
    assert.equal(h.statuses.at(-1), "wanted.standbyQueuedCancelFailed:1");
    assert.equal(h.refreshed(), 1);
  },
);

test(
  "dismissing the conflict dialog clears a stale error",
  { timeout: 5000 },
  async () => {
    const h = mount([
      {
        data: {
          queueStandbyRelease: { grabbed: false, conflicts: [conflict("one")] },
        },
      },
      { error: new Error("synthetic cancellation failure") },
      {
        data: {
          queueStandbyRelease: { grabbed: false, conflicts: [conflict("one")] },
        },
      },
    ]);
    await h.click("wanted.downloadStandby");
    await h.click("wanted.standbyCancelAndQueue");
    assert.ok(
      h.elements(h.render()).some((item) => item.props.role === "alert"),
    );
    const dismiss = h
      .elements(h.render())
      .find(
        (item) =>
          item.type === "button" &&
          [item.props.children].flat().includes("label.cancel"),
      );
    dismiss!.props.onClick();
    await h.click("wanted.downloadStandby");
    assert.ok(
      !h.elements(h.render()).some((item) => item.props.role === "alert"),
    );
  },
);
