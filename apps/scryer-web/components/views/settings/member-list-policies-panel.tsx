import { LoadingMark } from "@/components/common/loading-mark";
import { SingleSelectField } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListPolicy, MemberListPolicy } from "@/lib/types/lists";

const PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const TABLE_SHELL_CLASS =
  "overflow-hidden rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const TABLE_HEADER_ROW_CLASS =
  "border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]";
const TABLE_HEADER_CELL_CLASS = "font-semibold text-[var(--scry-muted2)]";

const POLICIES: readonly ListPolicy[] = ["AUTO", "APPROVAL", "NONE"];

type MemberListPoliciesPanelProps = {
  policies: MemberListPolicy[];
  loading: boolean;
  error: string | null;
  savingUserId: string | null;
  onChange: (userId: string, policy: ListPolicy) => void;
};

/** Per-member list policy with the member's recent list-request count, and nothing else. */
export function MemberListPoliciesPanel({
  policies,
  loading,
  error,
  savingUserId,
  onChange,
}: MemberListPoliciesPanelProps) {
  const t = useTranslate();
  return (
    <div id="member-list-policies" className={PANEL_CLASS}>
      <div className={PANEL_HEADER_CLASS}>
        <h3 className="text-[15px] font-semibold text-[var(--scry-ink2)]">{t("lists.policy.heading")}</h3>
        <p className="mt-0.5 text-[12.5px] text-[var(--scry-muted)]">{t("lists.policy.help")}</p>
      </div>
      <div className="p-4">
        {loading ? (
          <div className="flex items-center gap-2 text-sm text-[var(--scry-muted)]">
            <LoadingMark className="h-4 w-4" />
            {t("label.loading")}
          </div>
        ) : error ? (
          <p className="text-sm text-[var(--scry-danger-text)]">{error}</p>
        ) : policies.length === 0 ? (
          <p className="text-sm text-[var(--scry-muted)]">{t("lists.policy.empty")}</p>
        ) : (
          <div className={TABLE_SHELL_CLASS}>
            <Table>
              <TableHeader>
                <TableRow className={TABLE_HEADER_ROW_CLASS}>
                  <TableHead className={TABLE_HEADER_CELL_CLASS}>{t("lists.policy.member")}</TableHead>
                  <TableHead className={TABLE_HEADER_CELL_CLASS}>{t("lists.policy.requests30d")}</TableHead>
                  <TableHead className={TABLE_HEADER_CELL_CLASS}>{t("lists.policy.policy")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {policies.map((entry) => (
                  <TableRow key={entry.user.id} id={`member-list-policy-${entry.user.id}`}>
                    <TableCell className="font-medium text-[var(--scry-ink2)]">{entry.user.username}</TableCell>
                    <TableCell className="text-[var(--scry-muted)]">{entry.listRequestsLast30d}</TableCell>
                    <TableCell className="w-[220px]">
                      <SingleSelectField
                        id={`member-list-policy-select-${entry.user.id}`}
                        label={t("lists.policy.policyFor", { name: entry.user.username })}
                        labelClassName="sr-only"
                        className="space-y-0"
                        size="sm"
                        value={entry.policy}
                        options={POLICIES.map((policy) => ({
                          value: policy,
                          label: t(`lists.policy.value.${policy.toLowerCase()}`),
                        }))}
                        onValueChange={(value) => onChange(entry.user.id, value as ListPolicy)}
                        disabled={savingUserId === entry.user.id}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>
    </div>
  );
}
