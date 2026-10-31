export type ChmodPreset = {
  value: string;
  labelKey: string;
};

export const FOLDER_CHMOD_PRESETS: readonly ChmodPreset[] = [
  { value: "755", labelKey: "settings.chmodPresetOwnerWriteEveryoneRead" },
  { value: "775", labelKey: "settings.chmodPresetOwnerGroupWriteOtherRead" },
  { value: "770", labelKey: "settings.chmodPresetOwnerGroupWrite" },
  { value: "750", labelKey: "settings.chmodPresetOwnerWriteGroupRead" },
  { value: "777", labelKey: "settings.chmodPresetEveryoneWrite" },
] as const;

export const FILE_CHMOD_PRESETS: readonly ChmodPreset[] = [
  { value: "644", labelKey: "settings.chmodPresetOwnerWriteEveryoneRead" },
  { value: "664", labelKey: "settings.chmodPresetOwnerGroupWriteOtherRead" },
  { value: "660", labelKey: "settings.chmodPresetOwnerGroupWrite" },
  { value: "640", labelKey: "settings.chmodPresetOwnerWriteGroupRead" },
  { value: "666", labelKey: "settings.chmodPresetEveryoneWrite" },
] as const;

const CHMOD_MODE_BITS = [
  ["r", "w", "x"],
  ["r", "w", "x"],
  ["r", "w", "x"],
] as const;

export function isChmodPresetValue(
  presets: readonly ChmodPreset[],
  value: string,
): boolean {
  return presets.some((preset) => preset.value === value);
}

export function formatChmodMode(
  value: string,
  type: "file" | "folder",
): string | null {
  const normalized = value.trim();
  if (!/^[0-7]{3,4}$/.test(normalized)) {
    return null;
  }

  const modeDigits = normalized.slice(-3);
  const mode = modeDigits
    .split("")
    .map((digit, digitIndex) => {
      const bits = Number.parseInt(digit, 8);
      return CHMOD_MODE_BITS[digitIndex]
        .map((symbol, bitIndex) =>
          bits & (1 << (2 - bitIndex)) ? symbol : "-",
        )
        .join("");
    })
    .join("");

  return `${type === "folder" ? "d" : "-"}${mode}`;
}
