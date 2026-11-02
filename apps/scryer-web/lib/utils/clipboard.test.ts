import assert from "node:assert/strict";
import test from "node:test";

import { copyText } from "./clipboard.ts";

type Globals = { navigator?: unknown; document?: unknown };

function withGlobals(values: Globals, run: () => Promise<void>): Promise<void> {
  const saved = Object.getOwnPropertyDescriptors(globalThis);
  for (const [name, value] of Object.entries(values)) {
    Object.defineProperty(globalThis, name, {
      value,
      configurable: true,
      writable: true,
    });
  }
  return run().finally(() => {
    for (const name of Object.keys(values)) {
      const original = saved[name];
      if (original) {
        Object.defineProperty(globalThis, name, original);
      } else {
        delete (globalThis as Record<string, unknown>)[name];
      }
    }
  });
}

/** A document whose selection command copies the textarea's value into `taken`. */
function fakeDocument(taken: string[], outcome: boolean | "throw") {
  const children: Array<{ value: string }> = [];
  const focused = { focused: 0, focus: () => void focused.focused++ };
  return {
    activeElement: focused,
    body: {
      appendChild: (child: { value: string }) => void children.push(child),
    },
    createElement: () => {
      const area = {
        value: "",
        style: {},
        setAttribute: () => undefined,
        focus: () => undefined,
        select: () => undefined,
        setSelectionRange: () => undefined,
        remove: () => {
          children.splice(children.indexOf(area), 1);
        },
      };
      return area;
    },
    execCommand: (command: string) => {
      assert.equal(command, "copy");
      if (outcome === "throw") throw new Error("refused");
      if (outcome) taken.push(...children.map((child) => child.value));
      return outcome;
    },
    children,
    focused,
  };
}

test("a secure origin copies through the asynchronous clipboard", async () => {
  const written: string[] = [];
  const taken: string[] = [];
  await withGlobals(
    {
      navigator: {
        clipboard: {
          writeText: async (text: string) => void written.push(text),
        },
      },
      document: fakeDocument(taken, true),
    },
    async () => {
      assert.equal(await copyText("scryer_key"), true);
      assert.deepEqual(written, ["scryer_key"]);
      assert.deepEqual(taken, [], "no selection command runs");
    },
  );
});

test("a plain-HTTP origin, which has no asynchronous clipboard, copies through the selection command", async () => {
  const taken: string[] = [];
  const document = fakeDocument(taken, true);
  await withGlobals({ navigator: {}, document }, async () => {
    assert.equal(await copyText("scryer_key"), true);
    assert.deepEqual(taken, ["scryer_key"]);
    assert.deepEqual(
      document.children,
      [],
      "the throwaway textarea is removed",
    );
    assert.equal(
      document.focused.focused,
      1,
      "focus returns to the field that had it",
    );
  });
});

test("a refused asynchronous clipboard falls back to the selection command", async () => {
  const taken: string[] = [];
  const document = fakeDocument(taken, true);
  await withGlobals(
    {
      navigator: {
        clipboard: {
          writeText: async () => Promise.reject(new Error("not focused")),
        },
      },
      document,
    },
    async () => {
      assert.equal(await copyText("scryer_key"), true);
      assert.deepEqual(taken, ["scryer_key"]);
    },
  );
});

test("when neither path takes the text the copy reports false", async () => {
  await withGlobals({ navigator: {}, document: undefined }, async () => {
    assert.equal(await copyText("scryer_key"), false);
  });
  for (const outcome of [false, "throw"] as const) {
    const document = fakeDocument([], outcome);
    await withGlobals(
      {
        navigator: {
          clipboard: {
            writeText: async () => Promise.reject(new Error("not focused")),
          },
        },
        document,
      },
      async () => {
        assert.equal(await copyText("scryer_key"), false);
        assert.deepEqual(document.children, []);
      },
    );
  }
});
