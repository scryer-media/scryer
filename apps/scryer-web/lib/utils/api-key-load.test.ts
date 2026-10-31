import assert from "node:assert/strict";
import test from "node:test";
import { applyApiKeyLoadRefusal } from "./api-key-load.ts";

test("interactive refusal clears previously loaded keys regardless of language", () => {
  for (const message of [
    "An interactive session is required",
    "Interaktive Sitzung erforderlich",
  ]) {
    const state = {
      keys: ["existing-key"],
      canCreate: true,
      loaded: false,
      status: "old error" as string | null,
    };
    const setters = {
      setKeys: (keys: []) => {
        state.keys = keys;
      },
      setCanCreate: (allowed: boolean) => {
        state.canCreate = allowed;
      },
      setLoaded: (loaded: boolean) => {
        state.loaded = loaded;
      },
      setStatus: (status: string | null) => {
        state.status = status;
      },
    };
    const error = (code: string) => ({
      graphQLErrors: [
        {
          message,
          extensions: {
            code,
            reason: "INTERFACE_INTERACTIVE_SESSION_REQUIRED",
          },
        },
      ],
    });
    assert.equal(
      applyApiKeyLoadRefusal(error("VALIDATION_ERROR"), setters),
      false,
    );
    assert.deepEqual(state.keys, ["existing-key"]);
    assert.equal(applyApiKeyLoadRefusal(error("UNAUTHORIZED"), setters), true);
    assert.deepEqual(state, {
      keys: [],
      canCreate: false,
      loaded: true,
      status: null,
    });
  }
});
