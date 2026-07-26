"use client";

import { FormEvent, useState, useSyncExternalStore } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  StatusBadge,
  buttonClass,
  formatDate,
  inputClass,
  responseError,
  textareaClass,
} from "@/app/components/M7Primitives";
import { apiClient, type PatInfo } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import {
  SectionFeedback,
  SettingsListSkeleton,
  splitSettingList,
  subscribeRuntimeValue,
} from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

export function PatTokensCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState("server:read service:read task:* nat:* ddns:*");
  const hydratedPatExpiresAt = useHydratedPatExpiresAt();
  const [patExpiresAtOverride, setPatExpiresAt] = useState<string | null>(null);
  const patExpiresAt = patExpiresAtOverride ?? hydratedPatExpiresAt;
  const [patServerIds, setPatServerIds] = useState("");
  const [tokens, setTokens] = useState<PatInfo[]>([]);
  const [tokensLoading, setTokensLoading] = useState(true);
  const [createdToken, setCreatedToken] = useState("");
  const cardRef = useLoadOnVisible(() => void loadTokens());

  async function loadTokens() {
    setTokensLoading(true);
    const response = await apiClient.listPats();
    setTokensLoading(false);
    if (response.success && response.data) {
      setTokens(response.data);
    } else {
      setError(responseError(response));
    }
  }

  async function createToken(event: FormEvent) {
    event.preventDefault();
    const serverIds = splitSettingList(patServerIds);
    const expiresAt = new Date(patExpiresAt);
    if (Number.isNaN(expiresAt.getTime())) {
      setError(copy.settingsPage.enterValidPatExpiry);
      return;
    }
    if (serverIds.length === 0 && !(await dialogs.confirm({ message: copy.settingsPage.globalPatConfirm }))) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createPat({
      name,
      scopes: scopes.split(/\s+/).filter(Boolean),
      expires_at: expiresAt.toISOString(),
      ...(serverIds.length > 0 ? { server_ids: serverIds } : {}),
    }, totpCode);
    if (response.success && response.data) {
      setCreatedToken(response.data.token);
      setNotice(copy.settingsPage.patCreated.replace("{expiresAt}", String(formatDate(response.data.expires_at))));
      setName("");
      setPatExpiresAt(defaultPatExpiresAt());
      setPatServerIds("");
      await loadTokens();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <>
      <div ref={cardRef} className="grid gap-6 lg:grid-cols-2">
        <BrutalCard accent>
          <h2 className="mb-4 text-xl font-black uppercase">{copy.settingsPage.createPat}</h2>
          <SectionFeedback error={error} notice={notice} />
          <form onSubmit={createToken} className="space-y-4">
            <Field label={copy.settingsPage.nameLabel}><input className={inputClass} value={name} onChange={(e) => setName(e.target.value)} required /></Field>
            <Field label={copy.settingsPage.scopeLabel}><textarea className={`${textareaClass} min-h-24`} value={scopes} onChange={(e) => setScopes(e.target.value)} /></Field>
            <Field label={copy.settingsPage.expiryLabel}><input className={inputClass} type="datetime-local" value={patExpiresAt} onChange={(e) => setPatExpiresAt(e.target.value)} required /></Field>
            <Field label={copy.settingsPage.serverAllowlist}><textarea className={`${textareaClass} min-h-20`} value={patServerIds} onChange={(e) => setPatServerIds(e.target.value)} placeholder="server id, server id" /></Field>
            <button className={buttonClass("primary")}>{copy.settingsPage.createToken}</button>
          </form>
          {createdToken ? (
            <div className="mt-5 border-2 border-black bg-black p-3 font-mono text-xs text-green-300">
              {createdToken}
            </div>
          ) : null}
        </BrutalCard>

        <BrutalCard>
          <h2 className="mb-4 text-xl font-black uppercase">{copy.settingsPage.existingTokens}</h2>
          <div className="grid gap-3">
            {tokensLoading && tokens.length === 0 ? (
              <SettingsListSkeleton rows={2} />
            ) : tokens.length === 0 ? (
              <p className="text-sm font-bold text-[var(--text-muted)]">{copy.settingsPage.noTokens}</p>
            ) : (
              tokens.map((token) => {
                const serverIds = token.server_ids ?? [];
                return (
                  <div key={token.id} className="border-2 border-black bg-[var(--accent-bg)] p-3 text-sm font-bold">
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div className="min-w-0">
                        <div className="break-words text-base font-black">{token.name || copy.settingsPage.unnamedToken}</div>
                        <div className="mt-1 break-all font-mono text-[11px] text-[var(--text-muted)]">{token.id}</div>
                      </div>
                      <div className="flex flex-wrap gap-2">
                        {serverIds.length === 0 ? (
                          <StatusBadge tone="red">{copy.settingsPage.globalPat}</StatusBadge>
                        ) : (
                          <StatusBadge tone="green">{copy.settingsPage.serversCount.replace("{count}", String(serverIds.length))}</StatusBadge>
                        )}
                      </div>
                    </div>
                    <div className="mt-3 grid gap-2 text-xs text-[var(--text-muted)]">
                      <div>{copy.settingsPage.scopeValue.replace("{value}", String(token.scopes?.join(" ") || "N/A"))}</div>
                      <div>{copy.settingsPage.expiryValue.replace("{value}", String(formatDate(token.expires_at)))}</div>
                      <div>{copy.settingsPage.lastUsedValue.replace("{value}", String(formatDate(token.last_used_at)))}</div>
                      {serverIds.length > 0 ? <div className="break-all">{copy.settingsPage.serverAllowlistValue.replace("{value}", String(serverIds.join(", ")))}</div> : null}
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </BrutalCard>
      </div>
      {dialogs.element}
    </>
  );
}

let cachedClientPatExpiresAt: string | null = null;

// The default expiry depends on the client clock and timezone, so the server
// render stays empty and the value is filled in on hydration.
function useHydratedPatExpiresAt(): string {
  return useSyncExternalStore(
    subscribeRuntimeValue,
    () => {
      cachedClientPatExpiresAt ??= defaultPatExpiresAt();
      return cachedClientPatExpiresAt;
    },
    () => "",
  );
}

function defaultPatExpiresAt(): string {
  const date = new Date(Date.now() + 90 * 24 * 60 * 60 * 1000);
  const timezoneOffsetMs = date.getTimezoneOffset() * 60 * 1000;
  return new Date(date.getTime() - timezoneOffsetMs).toISOString().slice(0, 16);
}
