import { Check, X } from "lucide-react";

type RenderBooleanIconProps = {
  value: boolean;
  label: string;
};

export function RenderBooleanIcon({ value, label }: RenderBooleanIconProps) {
  return (
    <span
      className="inline-flex h-5 w-5 shrink-0 items-center justify-center"
      title={label}
      aria-label={label}
    >
      {value ? (
        <Check className="h-4 w-4 text-emerald-600 dark:text-emerald-300" />
      ) : (
        <X className="h-4 w-4 text-rose-600 dark:text-rose-300" />
      )}
    </span>
  );
}
