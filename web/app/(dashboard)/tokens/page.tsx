"use client";

import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  KpiCard,
  PageHeader,
  PageShell,
  ProgressBar,
  StatusBadge,
  TableSkeleton,
  buttonClass,
  compactId,
  formatDate,
  inputClass,
  responseError,
  tdClass,
  textareaClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, getApiBaseUrl, type PatInfo, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { showToast } from "@/app/components/Toasts";
import { escapeShellDoubleQuoted } from "@/app/lib/security";
import { useBusy } from "@/app/lib/use-busy";
import { useI18n } from "@/lib/use-i18n";

type CommandTab = "quick" | "daemon";
type TokenStatusFilter = "all" | "active" | "expired";
type ExpiryPreset = "24h" | "7d" | "30d";
type MaxUsesPreset = "1x" | "10x" | "Unlimited";
type TokenCreateKind = "enrollment" | "pat";

interface ManagedToken {
  id: string;
  name: string;
  tokenValue: string;
  hasRawSecret: boolean;
  kind: "enrollment" | "pat";
  used_count: number;
  max_uses: number | null;
  expires_at?: string | undefined;
  created_at?: string | undefined;
  last_used_at?: string | null | undefined;
  scopes?: string[] | undefined;
  server_ids?: string[] | null | undefined;
}

const STORED_ENROLLMENT_TOKENS_KEY = "xlstatus:enrollment-tokens:v1";
const HYDRATION_API_BASE_URL = (
  process.env["NEXT_PUBLIC_API_URL"]?.trim() || "http://localhost:8080"
).replace(/\/+$/, "");

const EXPIRY_PRESETS: readonly ExpiryPreset[] = ["24h", "7d", "30d"];
const MAX_USES_PRESETS: readonly MaxUsesPreset[] = ["1x", "10x", "Unlimited"];
const STATUS_FILTERS: readonly TokenStatusFilter[] = ["all", "active", "expired"];

function subscribeNoop(): () => void {
  return () => {};
}

function useRuntimeApiBaseUrl(): string {
  return useSyncExternalStore(
    subscribeNoop,
    getApiBaseUrl,
    () => HYDRATION_API_BASE_URL,
  );
}

function defaultGrpcUrl(apiBaseUrl: string): string {
  try {
    const url = new URL(apiBaseUrl);
    url.port = "50051";
    return url.toString().replace(/\/$/, "");
  } catch {
    return "http://localhost:50051";
  }
}

function expiryPresetToHours(preset: ExpiryPreset): number {
  if (preset === "7d") return 7 * 24;
  if (preset === "30d") return 30 * 24;
  return 24;
}

function maxUsesPresetToNumber(preset: MaxUsesPreset): number | null {
  if (preset === "1x") return 1;
  if (preset === "10x") return 10;
  return null;
}

function isTokenExpired(token: ManagedToken, nowMs: number): boolean {
  if (token.max_uses !== null && token.used_count >= token.max_uses) {
    return true;
  }
  if (!token.expires_at) return false;
  const expiresMs = new Date(token.expires_at).getTime();
  if (Number.isNaN(expiresMs)) return false;
  return expiresMs <= nowMs;
}

function readStoredEnrollmentTokens(): ManagedToken[] {
  if (typeof window === "undefined") return [];
  try {
    window.localStorage.removeItem(STORED_ENROLLMENT_TOKENS_KEY);
    const raw = window.sessionStorage.getItem(STORED_ENROLLMENT_TOKENS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item): item is ManagedToken =>
        Boolean(item && typeof item === "object" && typeof (item as ManagedToken).id === "string"),
    );
  } catch {
    return [];
  }
}

function writeStoredEnrollmentTokens(tokens: ManagedToken[]): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(STORED_ENROLLMENT_TOKENS_KEY);
    window.sessionStorage.setItem(STORED_ENROLLMENT_TOKENS_KEY, JSON.stringify(tokens));
  } catch {
    // Ignore storage quota errors in restricted browser contexts.
  }
}

function normalizePatToManagedToken(
  pat: PatInfo,
  secretById: Record<string, string>,
): ManagedToken {
  const rawRecord = pat as unknown as Record<string, unknown>;
  const rawUsedCount = rawRecord["used_count"];
  const rawMaxUses = rawRecord["max_uses"];
  const rawToken = rawRecord["token"];

  const used_count =
    typeof rawUsedCount === "number" && Number.isFinite(rawUsedCount)
      ? rawUsedCount
      : pat.last_used_at
        ? 1
        : 0;

  const max_uses =
    typeof rawMaxUses === "number" && Number.isFinite(rawMaxUses)
      ? rawMaxUses
      : rawMaxUses === null
        ? null
        : null;

  const knownSecret =
    secretById[pat.id] ?? (typeof rawToken === "string" && rawToken.trim() ? rawToken.trim() : "");

  return {
    id: pat.id,
    name: pat.name || `Token-${compactId(pat.id)}`,
    tokenValue: knownSecret || `xlp_${pat.id.replace(/-/g, "").slice(0, 16)}...`,
    hasRawSecret: Boolean(knownSecret),
    kind: "pat",
    used_count,
    max_uses,
    expires_at: pat.expires_at,
    created_at: pat.created_at,
    last_used_at: pat.last_used_at,
    scopes: pat.scopes,
    server_ids: pat.server_ids,
  };
}

export default function TokensPage() {
  const dialogs = useDialogs();
  const { locale, t: copy } = useI18n();
  const isEn = locale === "en-US";
  const runtimeApiBaseUrl = useRuntimeApiBaseUrl();

  const [pats, setPats] = useState<PatInfo[]>([]);
  const [enrollmentTokens, setEnrollmentTokens] = useState<ManagedToken[]>([]);
  const [patSecrets, setPatSecrets] = useState<Record<string, string>>({});
  const [nowMs, setNowMs] = useState<number>(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);

  // Studio state
  const [commandTab, setCommandTab] = useState<CommandTab>("quick");
  const [selectedTokenId, setSelectedTokenId] = useState<string>("");
  const [customNodeName, setCustomNodeName] = useState<string>("edge-node-01");
  const [serverUrlOverride, setServerUrlOverride] = useState<string | null>(null);
  const [grpcUrlOverride, setGrpcUrlOverride] = useState<string | null>(null);

  const serverUrl = serverUrlOverride ?? runtimeApiBaseUrl;
  const grpcUrl = grpcUrlOverride ?? defaultGrpcUrl(runtimeApiBaseUrl);

  // Creation drawer state
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [createKind, setCreateKind] = useState<TokenCreateKind>("enrollment");
  const [tokenLabel, setTokenLabel] = useState("");
  const [expiryPreset, setExpiryPreset] = useState<ExpiryPreset>("24h");
  const [maxUsesPreset, setMaxUsesPreset] = useState<MaxUsesPreset>("1x");
  const [patScopes, setPatScopes] = useState("server:read service:read task:* nat:* ddns:*");
  const [patServerAllowlist, setPatServerAllowlist] = useState("");

  // Table filter state
  const [statusFilter, setStatusFilter] = useState<TokenStatusFilter>("all");
  const [searchQuery, setSearchQuery] = useState("");

  const [formBusy, runForm] = useBusy();
  const [rowBusy, runRow] = useBusy();

  const loadTokens = useCallback(async () => {
    setLoading(true);
    setNowMs(Date.now());
    const storedEnrollments = readStoredEnrollmentTokens();
    setEnrollmentTokens(storedEnrollments);

    const response = await apiClient.listPats();
    setLoading(false);
    setNowMs(Date.now());
    if (response.success && response.data) {
      setPats(Array.isArray(response.data) ? response.data : []);
    } else {
      setError(responseError(response));
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadTokens();
  }, [loadTokens]);

  const allTokens = useMemo<ManagedToken[]>(() => {
    const normalizedPats = pats.map((pat) => normalizePatToManagedToken(pat, patSecrets));
    const combined = [...enrollmentTokens, ...normalizedPats];
    return combined.sort((a, b) => {
      const timeA = a.created_at ? new Date(a.created_at).getTime() : 0;
      const timeB = b.created_at ? new Date(b.created_at).getTime() : 0;
      return timeB - timeA;
    });
  }, [enrollmentTokens, pats, patSecrets]);

  const activeTokens = useMemo(
    () => allTokens.filter((token) => !isTokenExpired(token, nowMs)),
    [allTokens, nowMs],
  );

  // Automatically default to the newly created or first active token
  const selectedToken = useMemo<ManagedToken | null>(() => {
    if (selectedTokenId) {
      const match = allTokens.find((t) => t.id === selectedTokenId);
      if (match) return match;
    }
    return activeTokens[0] ?? allTokens[0] ?? null;
  }, [activeTokens, allTokens, selectedTokenId]);

  const effectiveTokenString = selectedToken?.tokenValue || "xle_<YOUR_ENROLLMENT_TOKEN>";
  const effectiveNodeName = customNodeName.trim()
    ? `"${escapeShellDoubleQuoted(customNodeName.trim())}"`
    : '"$(hostname)"';

  const quickEnrollCommand = useMemo(
    () =>
      `xlstatus-agent enroll --server "${escapeShellDoubleQuoted(serverUrl)}" --grpc-server "${escapeShellDoubleQuoted(grpcUrl)}" --token "${escapeShellDoubleQuoted(effectiveTokenString)}" --name ${effectiveNodeName}`,
    [effectiveNodeName, effectiveTokenString, grpcUrl, serverUrl],
  );

  const daemonRunCommand = useMemo(
    () => `${quickEnrollCommand} && xlstatus-agent run --config ./agent.yaml`,
    [quickEnrollCommand],
  );

  const activeStudioCommand = commandTab === "quick" ? quickEnrollCommand : daemonRunCommand;

  const kpiStats = useMemo(() => {
    const total = allTokens.length;
    const active = activeTokens.length;
    const expired = total - active;
    const totalUses = allTokens.reduce((sum, item) => sum + item.used_count, 0);
    return { total, active, expired, totalUses };
  }, [activeTokens.length, allTokens]);

  const deferredQuery = useDeferredValue(searchQuery);

  const filteredTokens = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase();
    return allTokens.filter((token) => {
      const expired = isTokenExpired(token, nowMs);
      if (statusFilter === "active" && expired) return false;
      if (statusFilter === "expired" && !expired) return false;
      if (!needle) return true;
      return [
        token.name,
        token.id,
        token.tokenValue,
        token.kind,
        ...(token.scopes ?? []),
      ].some((value) => value.toLowerCase().includes(needle));
    });
  }, [allTokens, deferredQuery, nowMs, statusFilter]);

  async function sensitiveTotpCode(): Promise<string | undefined | null> {
    let enabled = totpStatus?.enabled;
    if (totpStatus === null) {
      const response = await apiClient.getTotpStatus();
      if (!response.success || !response.data) {
        setError(responseError(response));
        return null;
      }
      setTotpStatus(response.data);
      enabled = response.data.enabled;
    }
    if (!enabled) return undefined;
    const code = await dialogs.totp();
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError(copy.settingsPage.totpEnterSixDigits);
      return null;
    }
    return trimmed;
  }

  async function handleCreateToken() {
    setError(null);
    const hours = expiryPresetToHours(expiryPreset);
    const maxUses = maxUsesPresetToNumber(maxUsesPreset);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;

    if (createKind === "enrollment") {
      // Backend caps enrollment token TTL at 24h; clamp for API call while preserving preset metadata
      const apiHours = Math.min(24, Math.max(1, hours));
      const response = await apiClient.createEnrollmentToken(apiHours, totpCode);
      if (response.success && response.data) {
        const created: ManagedToken = {
          id: `enr-${Date.now().toString(36)}`,
          name:
            tokenLabel.trim() ||
            `Enroll-${expiryPreset}-${maxUsesPreset}`,
          tokenValue: response.data.token,
          hasRawSecret: true,
          kind: "enrollment",
          used_count: 0,
          max_uses: maxUses,
          expires_at: response.data.expires_at,
          created_at: new Date().toISOString(),
          scopes: ["agent:enroll"],
        };
        const updated = [created, ...enrollmentTokens];
        setEnrollmentTokens(updated);
        writeStoredEnrollmentTokens(updated);
        setSelectedTokenId(created.id);
        setTokenLabel("");
        setDrawerOpen(false);
        showToast(copy.settingsPage.enrollmentTokenCreated);
      } else {
        setError(responseError(response));
      }
      return;
    }

    const serverIds = patServerAllowlist
      .split(/[\s,]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (
      serverIds.length === 0 &&
      !(await dialogs.confirm({ message: copy.settingsPage.globalPatConfirm }))
    ) {
      return;
    }

    const expiresAtIso = new Date(Date.now() + hours * 3600 * 1000).toISOString();
    const response = await apiClient.createPat(
      {
        name: tokenLabel.trim() || `PAT-${expiryPreset}`,
        scopes: patScopes.split(/\s+/).filter(Boolean),
        expires_at: expiresAtIso,
        ...(serverIds.length > 0 ? { server_ids: serverIds } : {}),
      },
      totpCode,
    );

    if (response.success && response.data) {
      const createdPat = response.data;
      setPatSecrets((prev) => ({ ...prev, [createdPat.id]: createdPat.token }));
      setSelectedTokenId(createdPat.id);
      setTokenLabel("");
      setPatServerAllowlist("");
      setDrawerOpen(false);
      showToast(
        copy.settingsPage.patCreated.replace(
          "{expiresAt}",
          String(formatDate(createdPat.expires_at)),
        ),
      );
      await loadTokens();
    } else {
      setError(responseError(response));
    }
  }

  async function handleRevokeToken(token: ManagedToken) {
    if (
      !(await dialogs.confirm({
        message: isEn
          ? `Revoke token "${token.name}"?`
          : `确定吊销并移除令牌「${token.name}」？`,
        danger: true,
      }))
    ) {
      return;
    }

    if (token.kind === "enrollment") {
      const next = enrollmentTokens.filter((item) => item.id !== token.id);
      setEnrollmentTokens(next);
      writeStoredEnrollmentTokens(next);
      showToast(isEn ? "Enrollment token removed." : "注册令牌已移除。");
      return;
    }

    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.revokePat(token.id, totpCode);
    if (response.success) {
      showToast(isEn ? "Token revoked." : "访问令牌已吊销。");
      await loadTokens();
    } else {
      setError(responseError(response));
    }
  }

  async function copyText(value: string, successMessage: string) {
    try {
      await navigator.clipboard.writeText(value);
      showToast(successMessage);
    } catch {
      setError(copy.settingsPage.clipboardCommandFailed);
    }
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={isEn ? "Onboarding & Credentials" : "节点接入与凭据"}
          title={isEn ? "Agent Onboarding Studio & Tokens" : "注册令牌与节点接入工作台"}
          detail={
            isEn
              ? "Generate enrollment tokens, build live CLI onboarding commands, and manage token lifecycles."
              : "交互式生成 Agent 注册命令、管理一次性/多次注册令牌与个人访问令牌生命周期。"
          }
          actions={
            <>
              <button
                type="button"
                className={buttonClass("secondary")}
                disabled={loading}
                onClick={() => void loadTokens()}
              >
                {loading ? copy.settingsPage.loading : copy.settingsPage.refresh}
              </button>
              <button
                type="button"
                className={buttonClass(drawerOpen ? "secondary" : "primary")}
                onClick={() => setDrawerOpen((prev) => !prev)}
                aria-expanded={drawerOpen}
              >
                {drawerOpen
                  ? isEn
                    ? "Close Drawer"
                    : "收起抽屉"
                  : "+ 生成注册令牌"}
              </button>
            </>
          }
        />

        {/* Top KPI Strip */}
        <section className="mb-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4" aria-label="Token KPIs">
          <KpiCard
            label={isEn ? "Total Tokens" : "令牌总数"}
            value={kpiStats.total}
            hint={isEn ? "Enrollment & PAT credentials" : "包含 Agent 注册令牌与 PAT"}
            badge={<StatusBadge tone="blue">Tokens</StatusBadge>}
          />
          <KpiCard
            label={isEn ? "Active Tokens" : "有效令牌"}
            value={kpiStats.active}
            hint={isEn ? "Ready for node onboarding" : "可用于新节点接入或 API 调用"}
            badge={
              <StatusBadge tone={kpiStats.active > 0 ? "green" : "gray"}>
                {isEn ? "Active" : "有效"}
              </StatusBadge>
            }
          />
          <KpiCard
            label={isEn ? "Expired / Exhausted" : "已过期 / 已耗尽"}
            value={kpiStats.expired}
            hint={isEn ? "Reached TTL or max uses" : "已达有效期或最大使用次数"}
            badge={
              <StatusBadge tone={kpiStats.expired > 0 ? "yellow" : "gray"}>
                {isEn ? "Expired" : "过期"}
              </StatusBadge>
            }
          />
          <KpiCard
            label={isEn ? "Total Onboarding Uses" : "累计调用 / 接入次数"}
            value={kpiStats.totalUses}
            hint={isEn ? "Tracked token invocations" : "已记录的令牌使用总次数"}
            badge={<StatusBadge tone="pink">Uses</StatusBadge>}
          />
        </section>

        <div className="mb-5 space-y-3">
          <InlineError message={error} />
        </div>

        {/* Interactive Onboarding Command Studio */}
        <BrutalCard accent className="mb-6">
          <div className="mb-4 flex flex-col gap-3 border-b border-[var(--border-color)] pb-4 lg:flex-row lg:items-center lg:justify-between">
            <div>
              <div className="flex flex-wrap items-center gap-2">
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {isEn ? "Interactive Onboarding Command Studio" : "交互式 Agent 接入命令工作台"}
                </h2>
                {selectedToken ? (
                  <StatusBadge tone="green">
                    {selectedToken.name} ({compactId(selectedToken.id)})
                  </StatusBadge>
                ) : (
                  <StatusBadge tone="yellow">
                    {isEn ? "No active token selected" : "未选择有效令牌"}
                  </StatusBadge>
                )}
              </div>
              <p className="mt-1 text-xs text-[var(--text-muted)]">
                {isEn
                  ? "Select any active token below or customize the node name to live-update the enrollment command."
                  : "从下方表格选择任意有效令牌并输入自定义节点名称，实时生成可一键复制的接入命令。"}
              </p>
            </div>

            {/* Command Tabs */}
            <div
              className="inline-flex items-center gap-1 self-start rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-1"
              role="tablist"
              aria-label="Onboarding Command Mode"
            >
              <button
                type="button"
                role="tab"
                aria-selected={commandTab === "quick"}
                onClick={() => setCommandTab("quick")}
                className={`rounded-lg px-3 py-1.5 text-xs font-medium transition ${
                  commandTab === "quick"
                    ? "bg-[var(--text-main)] text-[var(--bg-card)] shadow-[var(--shadow-xs)]"
                    : "text-[var(--text-muted)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-main)]"
                }`}
              >
                Quick Enroll CLI
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={commandTab === "daemon"}
                onClick={() => setCommandTab("daemon")}
                className={`rounded-lg px-3 py-1.5 text-xs font-medium transition ${
                  commandTab === "daemon"
                    ? "bg-[var(--text-main)] text-[var(--bg-card)] shadow-[var(--shadow-xs)]"
                    : "text-[var(--text-muted)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-main)]"
                }`}
              >
                Daemon Run
              </button>
            </div>
          </div>

          <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(0,1.25fr)]">
            <div className="grid gap-3 sm:grid-cols-2">
              <Field label={isEn ? "Custom Node Name" : "Custom Node Name (自定义节点名称)"}>
                <input
                  className={`${inputClass} font-mono`}
                  value={customNodeName}
                  onChange={(e) => setCustomNodeName(e.target.value)}
                  placeholder="edge-node-01"
                />
              </Field>
              <Field label={isEn ? "Selected Active Token" : "当前绑定令牌"}>
                <select
                  className={`${inputClass} font-mono`}
                  value={selectedToken?.id ?? ""}
                  onChange={(e) => setSelectedTokenId(e.target.value)}
                >
                  {allTokens.length === 0 ? (
                    <option value="">
                      {isEn ? "Generate a token first..." : "请先生成注册令牌..."}
                    </option>
                  ) : (
                    allTokens.map((token) => (
                      <option key={token.id} value={token.id}>
                        {token.name} ({token.kind === "enrollment" ? "Enroll" : "PAT"})
                      </option>
                    ))
                  )}
                </select>
              </Field>
              <Field label={copy.settingsPage.serverUrl}>
                <input
                  className={`${inputClass} font-mono`}
                  value={serverUrl}
                  onChange={(e) => setServerUrlOverride(e.target.value)}
                />
              </Field>
              <Field label={copy.settingsPage.grpcUrl}>
                <input
                  className={`${inputClass} font-mono`}
                  value={grpcUrl}
                  onChange={(e) => setGrpcUrlOverride(e.target.value)}
                />
              </Field>
            </div>

            <div className="flex flex-col justify-between rounded-xl border border-zinc-800 bg-zinc-950 p-4">
              <div className="mb-2 flex items-center justify-between gap-2">
                <span className="font-mono text-[11px] font-medium tracking-wider text-zinc-400 uppercase">
                  {commandTab === "quick" ? "Quick Enroll CLI" : "Daemon Run"}
                </span>
                <button
                  type="button"
                  className={buttonClass("primary")}
                  onClick={() =>
                    void copyText(
                      activeStudioCommand,
                      copy.settingsPage.agentCommandCopied,
                    )
                  }
                >
                  {isEn ? "Copy Command" : "一键复制命令"}
                </button>
              </div>
              <pre className="overflow-x-auto whitespace-pre-wrap break-all font-mono text-xs leading-relaxed text-emerald-400">
                {activeStudioCommand}
              </pre>
            </div>
          </div>
        </BrutalCard>

        {/* Collapsible Token Creation Drawer */}
        {drawerOpen ? (
          <BrutalCard className="mb-6 border-[var(--border-strong)]">
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3">
              <div>
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {isEn ? "Generate Onboarding / API Token" : "生成注册令牌"}
                </h2>
                <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                  {isEn
                    ? "Choose preset expiration and usage limits for rapid node enrollment."
                    : "选择预设有效期与最大使用次数，快速签发节点注册凭据。"}
                </p>
              </div>
              <button
                type="button"
                className={buttonClass("ghost")}
                onClick={() => setDrawerOpen(false)}
              >
                {copy.common.close}
              </button>
            </div>

            <form
              onSubmit={(event) => {
                event.preventDefault();
                void runForm(() => handleCreateToken());
              }}
              className="space-y-4"
            >
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <Field label={isEn ? "Token Type" : "令牌类型"}>
                  <div className="mt-1 grid grid-cols-2 gap-1.5 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1">
                    <button
                      type="button"
                      onClick={() => setCreateKind("enrollment")}
                      className={`rounded-md px-2.5 py-1.5 text-xs font-medium transition ${
                        createKind === "enrollment"
                          ? "bg-[var(--text-main)] text-[var(--bg-card)]"
                          : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {isEn ? "Enrollment" : "Agent 注册令牌"}
                    </button>
                    <button
                      type="button"
                      onClick={() => setCreateKind("pat")}
                      className={`rounded-md px-2.5 py-1.5 text-xs font-medium transition ${
                        createKind === "pat"
                          ? "bg-[var(--text-main)] text-[var(--bg-card)]"
                          : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {isEn ? "PAT Token" : "PAT 访问令牌"}
                    </button>
                  </div>
                </Field>

                <Field label={copy.settingsPage.nameLabel}>
                  <input
                    className={inputClass}
                    value={tokenLabel}
                    onChange={(e) => setTokenLabel(e.target.value)}
                    placeholder={isEn ? "hk-cluster-batch" : "例如：香港集群扩容批次"}
                  />
                </Field>

                {/* Preset Expiry Quick Chips */}
                <Field label={isEn ? "Preset Expiry" : "预设有效期"}>
                  <div className="mt-1 flex flex-wrap gap-1.5">
                    {EXPIRY_PRESETS.map((preset) => {
                      const active = expiryPreset === preset;
                      return (
                        <button
                          key={preset}
                          type="button"
                          onClick={() => setExpiryPreset(preset)}
                          className={`rounded-lg border px-3 py-1.5 font-mono text-xs font-medium transition ${
                            active
                              ? "border-transparent bg-[var(--text-main)] text-[var(--bg-card)]"
                              : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
                          }`}
                        >
                          {preset}
                        </button>
                      );
                    })}
                  </div>
                </Field>

                {/* Preset Max-Uses Quick Chips */}
                <Field label={isEn ? "Max Uses" : "最大使用次数"}>
                  <div className="mt-1 flex flex-wrap gap-1.5">
                    {MAX_USES_PRESETS.map((preset) => {
                      const active = maxUsesPreset === preset;
                      return (
                        <button
                          key={preset}
                          type="button"
                          onClick={() => setMaxUsesPreset(preset)}
                          className={`rounded-lg border px-3 py-1.5 font-mono text-xs font-medium transition ${
                            active
                              ? "border-transparent bg-[var(--text-main)] text-[var(--bg-card)]"
                              : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
                          }`}
                        >
                          {preset}
                        </button>
                      );
                    })}
                  </div>
                </Field>
              </div>

              {createKind === "pat" ? (
                <div className="grid gap-4 sm:grid-cols-2">
                  <Field label={copy.settingsPage.scopeLabel}>
                    <textarea
                      className={`${textareaClass} min-h-20`}
                      value={patScopes}
                      onChange={(e) => setPatScopes(e.target.value)}
                    />
                  </Field>
                  <Field label={copy.settingsPage.serverAllowlist}>
                    <textarea
                      className={`${textareaClass} min-h-20`}
                      value={patServerAllowlist}
                      onChange={(e) => setPatServerAllowlist(e.target.value)}
                      placeholder="server-uuid-1, server-uuid-2"
                    />
                  </Field>
                </div>
              ) : null}

              <div className="flex flex-wrap items-center justify-end gap-2 pt-1">
                <button
                  type="button"
                  className={buttonClass("secondary")}
                  onClick={() => setDrawerOpen(false)}
                >
                  {copy.common.cancel}
                </button>
                <button type="submit" disabled={formBusy} className={buttonClass("primary")}>
                  {isEn ? "Generate Token" : "立即生成令牌"}
                </button>
              </div>
            </form>
          </BrutalCard>
        ) : null}

        {/* Unified Filter Bar for Lifecycle Table */}
        <BrutalCard className="mb-5 p-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex flex-1 flex-col gap-3 sm:flex-row sm:items-center">
              <div className="min-w-0 flex-1">
                <input
                  type="search"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className={`${inputClass} mt-0`}
                  placeholder={
                    isEn
                      ? "Search token name, ID, prefix, or scope..."
                      : "搜索令牌名称、ID、前缀或权限范围..."
                  }
                  aria-label={copy.common.search}
                />
              </div>

              {/* Status Filter Pills (all / active / expired) */}
              <div
                className="inline-flex items-center gap-1 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1"
                role="group"
                aria-label="Token Status Filter"
              >
                {STATUS_FILTERS.map((status) => {
                  const active = statusFilter === status;
                  return (
                    <button
                      key={status}
                      type="button"
                      onClick={() => setStatusFilter(status)}
                      className={`rounded-lg px-2.5 py-1 font-mono text-xs font-medium transition ${
                        active
                          ? "bg-[var(--text-main)] text-[var(--bg-card)] shadow-[var(--shadow-xs)]"
                          : "text-[var(--text-muted)] hover:bg-[var(--bg-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {status}
                    </button>
                  );
                })}
              </div>
            </div>

            <button
              type="button"
              className={buttonClass(drawerOpen ? "secondary" : "primary")}
              onClick={() => setDrawerOpen((prev) => !prev)}
            >
              + 生成注册令牌
            </button>
          </div>
        </BrutalCard>

        {/* Token Lifecycle Table */}
        {loading && allTokens.length === 0 ? (
          <TableSkeleton columns={6} rows={4} />
        ) : filteredTokens.length === 0 ? (
          <EmptyState
            title={allTokens.length === 0 ? copy.settingsPage.noTokens : copy.common.noMatches}
            detail={
              allTokens.length === 0
                ? isEn
                  ? "Generate an enrollment token above to onboard new agent nodes."
                  : "点击「+ 生成注册令牌」签发首个节点注册凭据。"
                : isEn
                  ? "No tokens match the current filter."
                  : "没有符合当前筛选条件的令牌。"
            }
            action={
              allTokens.length === 0 ? (
                <button
                  type="button"
                  className={buttonClass("primary")}
                  onClick={() => setDrawerOpen(true)}
                >
                  + 生成注册令牌
                </button>
              ) : undefined
            }
          />
        ) : (
          <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]">
            <table className="w-full min-w-[62rem]">
              <thead>
                <tr>
                  <th className={thClass}>{copy.settingsPage.nameLabel}</th>
                  <th className={thClass}>{isEn ? "Type / Scope" : "类型与权限范围"}</th>
                  <th className={thClass}>
                    {isEn ? "Usage Progress (used_count / max_uses)" : "使用进度 (used_count / max_uses)"}
                  </th>
                  <th className={thClass}>{copy.settingsPage.expiryLabel}</th>
                  <th className={thClass}>{isEn ? "Status" : "状态"}</th>
                  <th className={thClass}>{isEn ? "Actions" : "操作"}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border-color)]">
                {filteredTokens.map((token) => {
                  const expired = isTokenExpired(token, nowMs);
                  const isSelected = selectedToken?.id === token.id;
                  const usagePercent =
                    token.max_uses && token.max_uses > 0
                      ? Math.min(100, Math.round((token.used_count / token.max_uses) * 100))
                      : token.used_count > 0
                        ? 25
                        : 0;
                  const usageLabel =
                    token.max_uses !== null
                      ? `${token.used_count} / ${token.max_uses}`
                      : `${token.used_count} / Unlimited`;

                  return (
                    <tr
                      key={token.id}
                      onClick={() => setSelectedTokenId(token.id)}
                      className={`cursor-pointer transition-colors ${
                        isSelected
                          ? "bg-[var(--bg-muted)]/70"
                          : "hover:bg-[var(--bg-subtle)]"
                      }`}
                    >
                      {/* Token Name & Secret Preview */}
                      <td className={tdClass}>
                        <div className="flex flex-col gap-1">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="font-semibold text-[var(--text-main)]">
                              {token.name}
                            </span>
                            {isSelected ? (
                              <StatusBadge tone="blue">
                                {isEn ? "In Studio" : "当前选用"}
                              </StatusBadge>
                            ) : null}
                          </div>
                          <span className="font-mono text-xs text-[var(--text-muted)]">
                            {token.hasRawSecret
                              ? `${token.tokenValue.slice(0, 14)}...`
                              : token.tokenValue}
                          </span>
                        </div>
                      </td>

                      {/* Type & Scopes */}
                      <td className={tdClass}>
                        <div className="flex flex-wrap items-center gap-1.5">
                          <StatusBadge tone={token.kind === "enrollment" ? "pink" : "blue"}>
                            {token.kind === "enrollment" ? "Enrollment" : "PAT"}
                          </StatusBadge>
                          <span className="font-mono text-xs text-[var(--text-muted)]">
                            {(token.scopes ?? []).slice(0, 2).join(" ") || "agent:enroll"}
                          </span>
                        </div>
                      </td>

                      {/* Usage Progress Bar (used_count / max_uses) */}
                      <td className={tdClass}>
                        <div className="w-44 space-y-1.5">
                          <div className="flex items-center justify-between font-mono text-xs">
                            <span className="font-medium text-[var(--text-main)]">{usageLabel}</span>
                            <span className="text-[var(--text-muted)]">
                              {token.max_uses !== null ? `${usagePercent}%` : "∞"}
                            </span>
                          </div>
                          <ProgressBar
                            value={usagePercent}
                            tone={expired ? "rose" : "emerald"}
                          />
                        </div>
                      </td>

                      {/* Expiry */}
                      <td className={tdClass}>
                        <div className="flex flex-col gap-0.5 font-mono text-xs text-[var(--text-muted)]">
                          <span>{formatDate(token.expires_at)}</span>
                          {token.last_used_at ? (
                            <span className="text-[11px]">
                              {copy.settingsPage.lastUsedValue.replace(
                                "{value}",
                                String(formatDate(token.last_used_at)),
                              )}
                            </span>
                          ) : null}
                        </div>
                      </td>

                      {/* Status */}
                      <td className={tdClass}>
                        <StatusBadge tone={expired ? "gray" : "green"}>
                          {expired ? "expired" : "active"}
                        </StatusBadge>
                      </td>

                      {/* Actions */}
                      <td className={tdClass} onClick={(e) => e.stopPropagation()}>
                        <div className="flex flex-wrap items-center gap-1.5">
                          <button
                            type="button"
                            className={buttonClass(isSelected ? "primary" : "secondary")}
                            onClick={() => setSelectedTokenId(token.id)}
                          >
                            {isEn ? "Use in Studio" : "载入工作台"}
                          </button>
                          {token.hasRawSecret ? (
                            <button
                              type="button"
                              className={buttonClass("secondary")}
                              onClick={() =>
                                void copyText(
                                  token.tokenValue,
                                  isEn ? "Token copied." : "令牌已复制。",
                                )
                              }
                            >
                              {isEn ? "Copy" : "复制令牌"}
                            </button>
                          ) : null}
                          <button
                            type="button"
                            className={buttonClass("danger")}
                            disabled={rowBusy}
                            onClick={() => void runRow(() => handleRevokeToken(token))}
                          >
                            {copy.settingsPage.revoke}
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </PageShell>
      {dialogs.element}
    </div>
  );
}
