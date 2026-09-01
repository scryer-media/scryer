import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import {
  CheckCircle2,
  Clipboard,
  KeyRound,
  Loader2,
  ShieldAlert,
  Trash2,
} from "lucide-react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SingleSelectField } from "@/components/ui/select";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import {
  createMyApiKeyMutation,
  revokeMyApiKeyMutation,
} from "@/lib/graphql/mutations";
import { myApiKeysQuery } from "@/lib/graphql/queries";
import type { ApiKeySummary, UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDateTime } from "@/lib/utils/date-format";

type MyApiKeysQueryResult = {
  canCreateMyApiKeys: boolean;
  myApiKeys: ApiKeySummary[];
};

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

function apiKeyStatus(key: ApiKeySummary, canCreate: boolean) {
  if (key.revokedAt) {
    return { label: "Revoked", tone: "negative" as const };
  }
  if (key.expiresAt && new Date(key.expiresAt).getTime() <= Date.now()) {
    return { label: "Expired", tone: "warning" as const };
  }
  if (!canCreate) {
    return { label: "Policy disabled", tone: "warning" as const };
  }
  return { label: "Active", tone: "positive" as const };
}

function formatTimestamp(value: string, dateTimeFormat: UiDateTimeFormat) {
  return formatUiDateTime(value, dateTimeFormat, { fallback: value });
}

export function ApiKeysPanel() {
  const client = useClient();
  const dateTimeFormat = useUiDateTimeFormat();
  const [keys, setKeys] = useState<ApiKeySummary[]>([]);
  const [canCreate, setCanCreate] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [label, setLabel] = useState("");
  const [expiry, setExpiry] = useState("DAYS_90");
  const [revealed, setRevealed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [revokingId, setRevokingId] = useState<string | null>(null);
  const [pendingRevoke, setPendingRevoke] = useState<ApiKeySummary | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const load = useCallback(async () => {
    const result = await client
      .query<MyApiKeysQueryResult>(myApiKeysQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (result.error || !result.data) {
      if (errorMessage(result.error, "").includes("interactive session is required")) {
        setKeys([]);
        setCanCreate(false);
        setLoaded(true);
        setStatus(null);
        return;
      }
      setLoaded(true);
      setStatus(errorMessage(result.error, "Unable to load API keys."));
      return;
    }
    setKeys(result.data.myApiKeys);
    setCanCreate(result.data.canCreateMyApiKeys);
    setLoaded(true);
    setStatus(null);
  }, [client]);

  useEffect(() => {
    void load();
  }, [load]);

  const create = useCallback(async () => {
    const trimmedLabel = label.trim();
    if (!trimmedLabel) {
      setStatus("Enter a name for this API key.");
      return;
    }
    setBusy(true);
    setStatus(null);
    try {
      const result = await client
        .mutation<{ createMyApiKey?: { apiKey: string; key: ApiKeySummary } }>(
          createMyApiKeyMutation,
          { input: { label: trimmedLabel, expiry } },
        )
        .toPromise();
      const created = result.data?.createMyApiKey;
      if (result.error || !created) {
        setStatus(errorMessage(result.error, "Unable to create API key."));
        return;
      }
      setRevealed(created.apiKey);
      setLabel("");
      await load();
    } finally {
      setBusy(false);
    }
  }, [client, expiry, label, load]);

  const copyRevealedKey = useCallback(async () => {
    if (!revealed) {
      return;
    }
    try {
      if (!navigator.clipboard) {
        throw new Error("Clipboard access is unavailable.");
      }
      await navigator.clipboard.writeText(revealed);
      setStatus("API key copied.");
    } catch (error) {
      setStatus(errorMessage(error, "Unable to copy the API key."));
    }
  }, [revealed]);

  const revoke = useCallback(async (id: string) => {
    setPendingRevoke(null);
    setRevokingId(id);
    setStatus(null);
    try {
      const result = await client.mutation(revokeMyApiKeyMutation, { id }).toPromise();
      if (result.error || !result.data?.revokeMyApiKey?.revoked) {
        setStatus(errorMessage(result.error, "Unable to revoke API key."));
        return;
      }
      await load();
    } finally {
      setRevokingId(null);
    }
  }, [client, load]);

  return (
    <>
      <section
        id="settings-profile-api-keys"
        className="mt-6 space-y-4 rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 shadow-[0_10px_24px_rgba(0,0,0,0.16)]"
      >
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <KeyRound
              className="h-4 w-4 text-[var(--scry-accent-text)]"
              aria-hidden="true"
            />
            <h3 className="text-base font-semibold text-[var(--scry-ink2)]">
              API keys
            </h3>
          </div>
        </div>

        {status ? (
          <div
            role="status"
            className="rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-2 text-sm text-[var(--scry-muted2)]"
          >
            {status}
          </div>
        ) : null}

        {revealed ? (
          <div className="space-y-3 rounded-[12px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-4">
            <div className="flex items-start gap-2">
              <ShieldAlert
                className="mt-0.5 h-4 w-4 shrink-0 text-[var(--scry-warning-text)]"
                aria-hidden="true"
              />
              <div>
                <p className="font-medium text-[var(--scry-warning-text)]">
                  Copy this key now
                </p>
                <p className="text-xs leading-5 text-[var(--scry-muted3)]">
                  For your security, Scryer will not display it again.
                </p>
              </div>
            </div>
            <code className="block break-all rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 py-2 font-[var(--font-code)] text-sm text-[var(--scry-ink2)]">
              {revealed}
            </code>
            <div className="flex flex-wrap gap-2">
              <Button type="button" onClick={() => void copyRevealedKey()}>
                <Clipboard aria-hidden="true" />
                Copy key
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => setRevealed(null)}
              >
                Dismiss
              </Button>
            </div>
          </div>
        ) : null}

        {canCreate ? (
          <form
            className="grid gap-4 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4 sm:grid-cols-[minmax(0,1fr)_minmax(180px,0.45fr)_auto] sm:items-end"
            onSubmit={(event) => {
              event.preventDefault();
              void create();
            }}
          >
            <div className="space-y-1.5">
              <Label
                htmlFor="settings-profile-api-key-name"
                className="min-h-5"
              >
                Name
              </Label>
              <Input
                id="settings-profile-api-key-name"
                value={label}
                onChange={(event) => setLabel(event.target.value)}
                maxLength={120}
                placeholder="e.g. Home automation"
                disabled={busy}
                className="h-10"
              />
            </div>
            <SingleSelectField
              id="settings-profile-api-key-expiry"
              label="Expires"
              value={expiry}
              onValueChange={setExpiry}
              disabled={busy}
              labelClassName="flex min-h-5 items-center leading-none"
              options={[
                { value: "DAYS_30", label: "30 days" },
                { value: "DAYS_90", label: "90 days" },
                { value: "DAYS_365", label: "1 year" },
                { value: "NEVER", label: "Never" },
              ]}
            />
            <div className="space-y-0 sm:space-y-1.5">
              <span
                aria-hidden="true"
                className="invisible hidden min-h-5 items-center text-sm font-medium leading-none sm:flex"
              >
                Action
              </span>
              <Button
                type="submit"
                disabled={busy || label.trim().length === 0}
                className="h-10 w-full sm:w-auto"
              >
                {busy ? (
                  <Loader2 className="animate-spin" aria-hidden="true" />
                ) : (
                  <KeyRound aria-hidden="true" />
                )}
                {busy ? "Creating…" : "Create API key"}
              </Button>
            </div>
          </form>
        ) : loaded ? (
          <div className="flex items-start gap-2 rounded-[12px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-4 text-sm text-[var(--scry-warning-text)]">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
            API key creation is disabled by the current security policy.
          </div>
        ) : null}

        {!loaded ? (
          <div className="flex items-center gap-2 py-2 text-sm text-[var(--scry-muted3)]">
            <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
            Loading API keys…
          </div>
        ) : keys.length === 0 ? (
          <div className="rounded-[12px] border border-dashed border-[var(--scry-border2)] px-4 py-6 text-center text-sm text-[var(--scry-muted3)]">
            No API keys yet.
          </div>
        ) : (
          <div className="space-y-3">
            {keys.map((key) => {
              const keyStatus = apiKeyStatus(key, canCreate);
              return (
                <div
                  key={key.id}
                  className="flex flex-col gap-4 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4 lg:flex-row lg:items-center lg:justify-between"
                >
                  <div className="min-w-0 flex-1 space-y-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium text-[var(--scry-ink2)]">
                        {key.label}
                      </span>
                      <Badge tone={keyStatus.tone}>{keyStatus.label}</Badge>
                      {key.provisioningSource === "environment" ? (
                        <Badge tone="info">Managed</Badge>
                      ) : null}
                    </div>
                    <dl className="grid gap-x-6 gap-y-2 text-xs sm:grid-cols-2 xl:grid-cols-4">
                      <div className="min-w-0">
                        <dt className="text-[var(--scry-muted3)]">Actor</dt>
                        <dd className="truncate font-[var(--font-code)] text-[var(--scry-ink2)]">
                          {key.actor}
                        </dd>
                      </div>
                      <div>
                        <dt className="text-[var(--scry-muted3)]">Created</dt>
                        <dd className="text-[var(--scry-ink2)]">
                          {formatTimestamp(key.createdAt, dateTimeFormat)}
                        </dd>
                      </div>
                      <div>
                        <dt className="text-[var(--scry-muted3)]">Expires</dt>
                        <dd className="text-[var(--scry-ink2)]">
                          {key.expiresAt
                            ? formatTimestamp(key.expiresAt, dateTimeFormat)
                            : "Never"}
                        </dd>
                      </div>
                      <div>
                        <dt className="text-[var(--scry-muted3)]">Last used</dt>
                        <dd className="text-[var(--scry-ink2)]">
                          {key.lastUsedAt
                            ? formatTimestamp(key.lastUsedAt, dateTimeFormat)
                            : "Never"}
                        </dd>
                      </div>
                    </dl>
                  </div>
                  {key.provisioningSource === "environment" ? (
                    <div className="flex items-center gap-1.5 text-xs text-[var(--scry-muted3)]">
                      <CheckCircle2 className="h-4 w-4" aria-hidden="true" />
                      Managed by Scryer
                    </div>
                  ) : !key.revokedAt ? (
                    <Button
                      type="button"
                      variant="destructive"
                      disabled={revokingId === key.id}
                      onClick={() => setPendingRevoke(key)}
                      className="w-fit"
                    >
                      {revokingId === key.id ? (
                        <Loader2 className="animate-spin" aria-hidden="true" />
                      ) : (
                        <Trash2 aria-hidden="true" />
                      )}
                      {revokingId === key.id ? "Revoking…" : "Revoke"}
                    </Button>
                  ) : null}
                </div>
              );
            })}
          </div>
        )}
      </section>

      <ConfirmDialog
        open={pendingRevoke !== null}
        title="Revoke API key?"
        description={
          pendingRevoke
            ? `“${pendingRevoke.label}” will stop working immediately. This cannot be undone.`
            : ""
        }
        confirmLabel="Revoke key"
        cancelLabel="Cancel"
        isBusy={pendingRevoke ? revokingId === pendingRevoke.id : false}
        onCancel={() => setPendingRevoke(null)}
        onConfirm={() => {
          if (pendingRevoke) {
            void revoke(pendingRevoke.id);
          }
        }}
      />
    </>
  );
}
