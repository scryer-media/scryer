export function readJsonString(rawValue?: string | null): string {
  if (!rawValue) {
    return "";
  }
  try {
    const parsed = JSON.parse(rawValue);
    if (typeof parsed === "string") {
      return parsed;
    }
    if (parsed === null) {
      return "";
    }
    if (typeof parsed === "object") {
      return JSON.stringify(parsed, null, 2);
    }
    return String(parsed);
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to parse JSON string payload", { rawValue, error });
    }
    return rawValue;
  }
}
