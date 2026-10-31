import * as React from "react";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { PROXY_FAMILY_LABEL_KEYS, groupProxiesByFamily } from "@/lib/types";
import type { ProxyRecord } from "@/lib/types";

/// The select's "no proxy" option. `null` cannot be a Radix item value, so the
/// two are translated at this boundary and nowhere else.
const NO_PROXY_VALUE = "none";

export type ProxyAssignmentSelectProps = {
  selectId: string;
  label: string;
  proxies: ProxyRecord[];
  value: string | null;
  onChange: (proxyConfigId: string | null) => void;
  disabled?: boolean;
  /** Hide the visual label when the surrounding table supplies the header. */
  showLabel?: boolean;
  /** Keep the control to one table-cell line without secondary guidance. */
  compact?: boolean;
  /** Extra guidance rendered under the select, above any warning. */
  helpText?: string;
};

/**
 * The proxy a consumer is assigned, grouped by family. Indexers and download
 * clients pick from the same list — any kind may be chosen — so the option
 * list, the "assigned but missing" case and the disabled-proxy warning live
 * here rather than being written twice.
 */
export function ProxyAssignmentSelect({
  selectId,
  label,
  proxies,
  value,
  onChange,
  disabled,
  showLabel = true,
  compact = false,
  helpText,
}: ProxyAssignmentSelectProps) {
  const t = useTranslate();
  const assigned = value
    ? (proxies.find((proxy) => proxy.id === value) ?? null)
    : null;
  const isMissing = Boolean(value) && !assigned;
  // A disabled proxy is still shown while it is the assignment, so the operator
  // can see what is set instead of the select silently reading as "direct".
  const selectable = React.useMemo(
    () => proxies.filter((proxy) => proxy.isEnabled || proxy.id === value),
    [proxies, value],
  );
  const groups = React.useMemo(
    () => groupProxiesByFamily(selectable),
    [selectable],
  );

  return (
    <div className={compact ? "min-w-0" : "space-y-2"}>
      <Label className={showLabel ? "block" : "sr-only"} htmlFor={selectId}>
        {label}
      </Label>
      <Select
        value={value ?? NO_PROXY_VALUE}
        disabled={disabled}
        onValueChange={(next) =>
          onChange(next === NO_PROXY_VALUE ? null : next)
        }
      >
        <SelectTrigger
          id={selectId}
          className={compact ? "w-full max-w-36" : "w-full"}
        >
          <SelectValue placeholder={t("settings.proxyDirect")} />
        </SelectTrigger>
        <SelectContent
          position="popper"
          className="w-80 max-w-[var(--radix-select-content-available-width)]"
        >
          <SelectItem value={NO_PROXY_VALUE}>
            {t("settings.proxyDirect")}
          </SelectItem>
          {isMissing ? (
            <SelectItem value={value ?? "missing"} disabled>
              {t("settings.proxyMissing")}
            </SelectItem>
          ) : null}
          {groups.map((group) => (
            <SelectGroup key={group.family ?? "other"}>
              <SelectLabel>
                {group.family
                  ? t(PROXY_FAMILY_LABEL_KEYS[group.family])
                  : t("settings.proxyFamilyOther")}
              </SelectLabel>
              {group.proxies.map((proxy) => (
                <SelectItem key={proxy.id} value={proxy.id}>
                  {proxy.isEnabled
                    ? proxy.name
                    : `${proxy.name} ${t("settings.proxyDisabledSuffix")}`}
                </SelectItem>
              ))}
            </SelectGroup>
          ))}
        </SelectContent>
      </Select>
      {!compact && helpText ? (
        <p className="text-xs text-muted-foreground">{helpText}</p>
      ) : null}
      {!compact && isMissing ? (
        <p className="text-xs text-[var(--scry-warning-text)]">
          {t("settings.proxyMissingHelp")}
        </p>
      ) : !compact && assigned && !assigned.isEnabled ? (
        <p className="text-xs text-[var(--scry-warning-text)]">
          {t("settings.proxyDisabledHelp")}
        </p>
      ) : null}
    </div>
  );
}
