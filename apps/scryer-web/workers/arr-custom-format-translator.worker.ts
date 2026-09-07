/// <reference lib="webworker" />

import { translateCustomFormats } from "@/lib/arr-custom-format/translate";
import type { ImportedCustomFormat, TranslationResult } from "@/lib/arr-custom-format/types";

type TranslationRequest = {
  type: "translate";
  requestId: number;
  formats: ImportedCustomFormat[];
  scores: Record<string, number>;
};

type TranslationResponse =
  | { type: "complete"; requestId: number; result: TranslationResult }
  | { type: "error"; requestId: number; message: string };

self.onmessage = (event: MessageEvent<TranslationRequest>) => {
  const request = event.data;
  if (request.type !== "translate") return;

  try {
    const result = translateCustomFormats(request.formats, request.scores);
    self.postMessage({
      type: "complete",
      requestId: request.requestId,
      result,
    } satisfies TranslationResponse);
  } catch (error) {
    self.postMessage({
      type: "error",
      requestId: request.requestId,
      message: error instanceof Error ? error.message : "Translation failed",
    } satisfies TranslationResponse);
  }
};
