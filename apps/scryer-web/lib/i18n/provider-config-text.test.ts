import assert from "node:assert/strict";
import test from "node:test";
import { translateProviderConfigText } from "./provider-config-text.ts";
import ru from "./locales/ru.ts";
const t = (key: string) => ru[key] ?? key;
test("known provider schema labels use the selected UI dictionary", () => {
  assert.equal(
    translateProviderConfigText("Known provider", t),
    "Известный провайдер",
  );
  assert.equal(translateProviderConfigText("Base URL", t), "Базовый URL");
});
