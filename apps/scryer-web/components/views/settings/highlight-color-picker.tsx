import { useMemo, useState } from "react";
import {
  HslColorPicker,
  RgbColorPicker,
  type HslColor,
  type RgbColor,
} from "react-colorful";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";

type ColorMode = "rgb" | "hsl";

type Props = {
  value: string;
  onPreview: (value: string) => void;
  onApply: (value: string) => void;
  onCancel: () => void;
};

const clamp = (value: number, minimum: number, maximum: number) =>
  Math.min(maximum, Math.max(minimum, value));

function hexToRgb(value: string): RgbColor {
  const normalized = value.replace("#", "");
  return {
    r: Number.parseInt(normalized.slice(0, 2), 16),
    g: Number.parseInt(normalized.slice(2, 4), 16),
    b: Number.parseInt(normalized.slice(4, 6), 16),
  };
}

function rgbToHex({ r, g, b }: RgbColor): string {
  const channel = (value: number) =>
    Math.round(clamp(value, 0, 255)).toString(16).padStart(2, "0");
  return `#${channel(r)}${channel(g)}${channel(b)}`;
}

function rgbToHsl({ r, g, b }: RgbColor): HslColor {
  const red = r / 255;
  const green = g / 255;
  const blue = b / 255;
  const maximum = Math.max(red, green, blue);
  const minimum = Math.min(red, green, blue);
  const lightness = (maximum + minimum) / 2;
  const difference = maximum - minimum;

  if (difference === 0) {
    return { h: 0, s: 0, l: Math.round(lightness * 100) };
  }

  const saturation =
    difference / (1 - Math.abs(2 * lightness - 1));
  let hue: number;
  if (maximum === red) {
    hue = 60 * (((green - blue) / difference) % 6);
  } else if (maximum === green) {
    hue = 60 * ((blue - red) / difference + 2);
  } else {
    hue = 60 * ((red - green) / difference + 4);
  }

  return {
    h: Math.round(hue < 0 ? hue + 360 : hue),
    s: Math.round(saturation * 100),
    l: Math.round(lightness * 100),
  };
}

function hslToRgb({ h, s, l }: HslColor): RgbColor {
  const hue = ((h % 360) + 360) % 360;
  const saturation = clamp(s, 0, 100) / 100;
  const lightness = clamp(l, 0, 100) / 100;
  const chroma = (1 - Math.abs(2 * lightness - 1)) * saturation;
  const segment = hue / 60;
  const secondary = chroma * (1 - Math.abs((segment % 2) - 1));
  const offset = lightness - chroma / 2;

  let red = 0;
  let green = 0;
  let blue = 0;
  if (segment < 1) {
    red = chroma;
    green = secondary;
  } else if (segment < 2) {
    red = secondary;
    green = chroma;
  } else if (segment < 3) {
    green = chroma;
    blue = secondary;
  } else if (segment < 4) {
    green = secondary;
    blue = chroma;
  } else if (segment < 5) {
    red = secondary;
    blue = chroma;
  } else {
    red = chroma;
    blue = secondary;
  }

  return {
    r: Math.round((red + offset) * 255),
    g: Math.round((green + offset) * 255),
    b: Math.round((blue + offset) * 255),
  };
}

function ColorChannelInput({
  id,
  label,
  value,
  maximum,
  suffix,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  maximum: number;
  suffix?: string;
  onChange: (value: number) => void;
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id} className="text-xs text-[var(--scry-muted2)]">
        {label}
      </Label>
      <div className="relative">
        <Input
          id={id}
          type="text"
          inputMode="numeric"
          value={Math.round(value)}
          onChange={(event) => {
            const nextValue = event.target.value.trim();
            if (/^\d{1,3}$/.test(nextValue)) {
              onChange(clamp(Number(nextValue), 0, maximum));
            }
          }}
          className={suffix ? "pr-7" : undefined}
        />
        {suffix ? (
          <span className="pointer-events-none absolute inset-y-0 right-2.5 flex items-center text-xs text-[var(--scry-muted3)]">
            {suffix}
          </span>
        ) : null}
      </div>
    </div>
  );
}

export default function HighlightColorPicker({
  value,
  onPreview,
  onApply,
  onCancel,
}: Props) {
  const t = useTranslate();
  const [mode, setMode] = useState<ColorMode>("rgb");
  const [rgb, setRgb] = useState<RgbColor>(() => hexToRgb(value));
  const hsl = useMemo(() => rgbToHsl(rgb), [rgb]);
  const hex = rgbToHex(rgb);

  const updateRgb = (nextValue: RgbColor) => {
    setRgb(nextValue);
    onPreview(rgbToHex(nextValue));
  };
  const updateRgbChannel = (channel: keyof RgbColor, nextValue: number) => {
    updateRgb({ ...rgb, [channel]: nextValue });
  };
  const updateHslChannel = (channel: keyof HslColor, nextValue: number) => {
    updateRgb(hslToRgb({ ...hsl, [channel]: nextValue }));
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="font-medium text-[var(--scry-ink2)]">
            {t("profile.highlightColorCustom")}
          </div>
          <div className="font-mono text-xs uppercase text-[var(--scry-muted2)]">
            {hex}
          </div>
        </div>
        <div
          className="flex rounded-lg border border-[var(--scry-border2)] bg-[var(--scry-inset)] p-0.5"
          role="group"
          aria-label={t("profile.highlightColorModel")}
        >
          {(["rgb", "hsl"] as const).map((candidate) => (
            <Button
              key={candidate}
              type="button"
              size="xs"
              variant={mode === candidate ? "secondary" : "ghost"}
              aria-pressed={mode === candidate}
              onClick={() => setMode(candidate)}
              className="min-w-11 uppercase"
            >
              {candidate}
            </Button>
          ))}
        </div>
      </div>

      {mode === "rgb" ? (
        <RgbColorPicker
          color={rgb}
          onChange={updateRgb}
          style={{ width: "100%", height: 190 }}
        />
      ) : (
        <HslColorPicker
          color={hsl}
          onChange={(nextColor) => updateRgb(hslToRgb(nextColor))}
          style={{ width: "100%", height: 190 }}
        />
      )}

      <div className="grid grid-cols-3 gap-2">
        {mode === "rgb" ? (
          <>
            <ColorChannelInput
              id="settings-profile-highlight-r"
              label="R"
              value={rgb.r}
              maximum={255}
              onChange={(nextValue) => updateRgbChannel("r", nextValue)}
            />
            <ColorChannelInput
              id="settings-profile-highlight-g"
              label="G"
              value={rgb.g}
              maximum={255}
              onChange={(nextValue) => updateRgbChannel("g", nextValue)}
            />
            <ColorChannelInput
              id="settings-profile-highlight-b"
              label="B"
              value={rgb.b}
              maximum={255}
              onChange={(nextValue) => updateRgbChannel("b", nextValue)}
            />
          </>
        ) : (
          <>
            <ColorChannelInput
              id="settings-profile-highlight-h"
              label="H"
              value={hsl.h}
              maximum={360}
              suffix="°"
              onChange={(nextValue) => updateHslChannel("h", nextValue)}
            />
            <ColorChannelInput
              id="settings-profile-highlight-s"
              label="S"
              value={hsl.s}
              maximum={100}
              suffix="%"
              onChange={(nextValue) => updateHslChannel("s", nextValue)}
            />
            <ColorChannelInput
              id="settings-profile-highlight-l"
              label="L"
              value={hsl.l}
              maximum={100}
              suffix="%"
              onChange={(nextValue) => updateHslChannel("l", nextValue)}
            />
          </>
        )}
      </div>

      <div className="flex justify-end gap-2 border-t border-[var(--scry-border)] pt-3">
        <Button type="button" variant="outline" size="sm" onClick={onCancel}>
          {t("profile.highlightColorCancel")}
        </Button>
        <Button type="button" size="sm" onClick={() => onApply(hex)}>
          {t("profile.highlightColorApply")}
        </Button>
      </div>
    </div>
  );
}
