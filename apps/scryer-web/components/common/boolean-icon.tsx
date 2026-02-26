import { Check, X } from "lucide-react";

type RenderBooleanIconProps = {
  value: boolean;
  label: string;
};

export function RenderBooleanIcon({ value, label }: RenderBooleanIconProps) {
  return (
    <span
      className={`inline-flex h-7 w-7 items-center justify-center rounded border ${
        value ? "border-emerald-300 bg-emerald-50 dark:bg-emerald-950" : "border-rose-500 bg-rose-950"
      }`}
      title={label}
      aria-label={label}
    >
      {value ? <Check className="h-4 w-4 text-emerald-700 dark:text-emerald-300" /> : <X className="h-4 w-4 text-rose-300" />}
    </span>
  );
}
