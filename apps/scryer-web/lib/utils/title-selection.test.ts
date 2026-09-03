import assert from "node:assert/strict";
import test, { after, before } from "node:test";
import { fileURLToPath } from "node:url";
import { createServer, type ViteDevServer } from "vite";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));

type TitleSelectionModule = {
  selectedTitleIdsKey: (titles: readonly { id: string }[]) => string;
};

let server: ViteDevServer;
let titleSelection: TitleSelectionModule;

before(async () => {
  server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });
  titleSelection = (await server.ssrLoadModule(
    "/lib/utils/title-selection.ts",
  )) as unknown as TitleSelectionModule;
});

after(async () => {
  await server.close();
});

function titles(...ids: string[]): { id: string }[] {
  return ids.map((id) => ({ id }));
}

test("regenerated title objects with the same ids key identically", () => {
  const first = titles("title-a", "title-b", "title-c");
  const second = titles("title-a", "title-b", "title-c");
  assert.notStrictEqual(first[0], second[0]);
  assert.equal(
    titleSelection.selectedTitleIdsKey(first),
    titleSelection.selectedTitleIdsKey(second),
  );
});

test("selection order does not affect the key", () => {
  assert.equal(
    titleSelection.selectedTitleIdsKey(titles("title-c", "title-a", "title-b")),
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-b", "title-c")),
  );
});

test("an added title changes the key", () => {
  assert.notEqual(
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-b")),
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-b", "title-c")),
  );
});

test("a removed title changes the key", () => {
  assert.notEqual(
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-b", "title-c")),
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-c")),
  );
});

test("a swapped title changes the key", () => {
  assert.notEqual(
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-b")),
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-z")),
  );
});

test("an empty selection keys to the empty string", () => {
  assert.equal(titleSelection.selectedTitleIdsKey([]), "");
});

test("a single selection is not confused with a longer one sharing its prefix", () => {
  assert.notEqual(
    titleSelection.selectedTitleIdsKey(titles("title-a")),
    titleSelection.selectedTitleIdsKey(titles("title-ab")),
  );
});

test("duplicate ids collapse so the key describes the id set", () => {
  assert.equal(
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-a", "title-b")),
    titleSelection.selectedTitleIdsKey(titles("title-a", "title-b")),
  );
});
