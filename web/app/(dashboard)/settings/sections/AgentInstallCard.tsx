"use client";

import { useEffect, useState, useSyncExternalStore } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  buttonClass,
  inputClass,
  responseError,
} from "@/app/components/M7Primitives";
import { showToast } from "@/app/components/Toasts";
import { encodePathSegment } from "@/app/lib/security";
import { apiClient, getApiBaseUrl } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { useBusy } from "@/app/lib/use-busy";
import { SectionFeedback, subscribeRuntimeValue } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

const DEFAULT_AGENT_VERSION = "v0.3";
const GITHUB_RELEASES_API = "https://api.github.com/repos/lbyxiaolizi/XLStatus/releases?per_page=20";
const GITHUB_RELEASE_CACHE_KEY = "xlstatus:agent-latest-release:v1";
const GITHUB_RELEASE_CACHE_TTL_MS = 45 * 60 * 1000;
const HYDRATION_API_BASE_URL = (process.env["NEXT_PUBLIC_API_URL"]?.trim() || "http://localhost:8080").replace(/\/+$/, "");

// First card on the page, so it loads eagerly instead of waiting for the
// viewport observer used by the cards below it.
export function AgentInstallCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const runtimeApiBaseUrl = useRuntimeApiBaseUrl();
  const [agentServerUrlOverride, setAgentServerUrl] = useState<string | null>(null);
  const [agentGrpcUrlOverride, setAgentGrpcUrl] = useState<string | null>(null);
  const agentServerUrl = agentServerUrlOverride ?? runtimeApiBaseUrl;
  const agentGrpcUrl = agentGrpcUrlOverride ?? defaultGrpcUrl(runtimeApiBaseUrl);
  const [agentName, setAgentName] = useState("$(hostname)");
  const [agentVersion, setAgentVersion] = useState(DEFAULT_AGENT_VERSION);
  const [agentUseLatestVersion, setAgentUseLatestVersion] = useState(true);
  const [agentLatestVersionStatus, setAgentLatestVersionStatus] = useState(copy.settingsPage.agentVersionWaiting);
  const [agentLatestVersionLoading, setAgentLatestVersionLoading] = useState(false);
  const [enrollmentHours, setEnrollmentHours] = useState("1");
  const [enrollmentToken, setEnrollmentToken] = useState("");
  const [enrollmentExpiresAt, setEnrollmentExpiresAt] = useState("");
  const installScriptUrl = apiClient.getAgentInstallScriptUrl({
    server_url: agentServerUrl,
    grpc_server: agentGrpcUrl,
    enrollment_token: enrollmentToken || "xle_...",
    agent_name: agentName,
    version: agentVersion,
  }, runtimeApiBaseUrl);
  const githubScriptUrl = `https://github.com/lbyxiaolizi/XLStatus/releases/download/${encodePathSegment(agentVersion)}/install-agent.sh`;
  const agentInstallCommand = buildAgentInstallCommand({
    installScriptUrl,
  });

  useEffect(() => {
    if (!agentUseLatestVersion) return;
    return scheduleWhenIdle(() => void refreshLatestAgentVersion());
    // The refresh function intentionally runs only when the auto-latest switch changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentUseLatestVersion]);

  async function createEnrollmentToken() {
    const expiresInHours = Number.parseInt(enrollmentHours, 10);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createEnrollmentToken(
      Number.isFinite(expiresInHours) && expiresInHours > 0 ? expiresInHours : 1,
      totpCode,
    );
    if (response.success && response.data) {
      setEnrollmentToken(response.data.token);
      setEnrollmentExpiresAt(response.data.expires_at);
      showToast(copy.settingsPage.enrollmentTokenCreated);
    } else {
      setError(responseError(response));
    }
  }

  async function refreshLatestAgentVersion({ forceNetwork = false }: { forceNetwork?: boolean } = {}) {
    if (!forceNetwork) {
      const cachedVersion = readCachedLatestAgentVersion();
      if (cachedVersion) {
        setAgentVersion(cachedVersion);
        setAgentLatestVersionStatus(copy.settingsPage.agentLatestFetched.replace("{tag}", String(cachedVersion)));
        return;
      }
    }

    setAgentLatestVersionLoading(true);
    try {
      const response = await fetch(GITHUB_RELEASES_API, {
        headers: { Accept: "application/vnd.github+json" },
        cache: forceNetwork ? "no-store" : "default",
      });
      if (!response.ok) {
        throw new Error(copy.settingsPage.githubHttpError.replace("{status}", String(response.status)));
      }
      const data = (await response.json()) as Array<{ tag_name?: string; draft?: boolean }>;
      const release = data.find((item) => !item.draft && item.tag_name?.trim());
      const tagName = release?.tag_name?.trim();
      if (!tagName) {
        throw new Error(copy.settingsPage.githubNoTagName);
      }
      cacheLatestAgentVersion(tagName);
      setAgentVersion(tagName);
      setAgentLatestVersionStatus(copy.settingsPage.agentLatestFetched.replace("{tag}", String(tagName)));
    } catch (err) {
      const message = err instanceof Error ? err.message : copy.settingsPage.unknownError;
      setAgentLatestVersionStatus(
        copy.settingsPage.agentLatestFetchFailed
          .replace("{version}", String(agentVersion || DEFAULT_AGENT_VERSION))
          .replace("{message}", String(message)),
      );
      if (!agentVersion.trim()) {
        setAgentVersion(DEFAULT_AGENT_VERSION);
      }
    } finally {
      setAgentLatestVersionLoading(false);
    }
  }

  async function copyAgentCommand() {
    try {
      await navigator.clipboard.writeText(agentInstallCommand);
      showToast(copy.settingsPage.agentCommandCopied);
    } catch {
      setError(copy.settingsPage.clipboardCommandFailed);
    }
  }

  return (
    <>
      <BrutalCard accent className="mb-6">
        <SectionFeedback error={error} />
        <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
          <div>
            <h2 className="mb-4 text-lg font-semibold tracking-tight text-[var(--text-main)]">{copy.settingsPage.agentInstall}</h2>
            <div className="grid gap-4 md:grid-cols-2">
              <Field label={copy.settingsPage.serverUrl}>
                <input className={inputClass} value={agentServerUrl} onChange={(e) => setAgentServerUrl(e.target.value)} />
              </Field>
              <Field label={copy.settingsPage.grpcUrl}>
                <input className={inputClass} value={agentGrpcUrl} onChange={(e) => setAgentGrpcUrl(e.target.value)} />
              </Field>
              <Field label={copy.settingsPage.releaseVersion}>
                <input
                  className={inputClass}
                  value={agentVersion}
                  onChange={(e) => setAgentVersion(e.target.value)}
                  disabled={agentUseLatestVersion}
                />
              </Field>
              <Field label={copy.settingsPage.agentName}>
                <input className={inputClass} value={agentName} onChange={(e) => setAgentName(e.target.value)} />
              </Field>
              <Field label={copy.settingsPage.enrollmentHours}>
                <input className={inputClass} type="number" min="1" max="24" value={enrollmentHours} onChange={(e) => setEnrollmentHours(e.target.value)} />
              </Field>
            </div>
            <div className="mt-4 flex flex-wrap items-center gap-2">
              <label className="inline-flex cursor-pointer items-center gap-2 rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-2 text-xs font-medium text-[var(--text-main)]">
                <input
                  type="checkbox"
                  checked={agentUseLatestVersion}
                  onChange={(event) => setAgentUseLatestVersion(event.target.checked)}
                />
                {copy.settingsPage.fetchLatestFromGithub}
              </label>
              <button
                type="button"
                className={buttonClass("secondary")}
                onClick={() => void refreshLatestAgentVersion({ forceNetwork: true })}
                disabled={agentLatestVersionLoading}
              >
                {agentLatestVersionLoading ? copy.settingsPage.fetching : copy.settingsPage.refreshVersion}
              </button>
            </div>
            <p className="mt-2 text-xs font-medium text-[var(--text-muted)]">
              {agentLatestVersionStatus}
            </p>
            <div className="mt-4 flex flex-wrap gap-2">
              <button type="button" className={buttonClass("primary")} onClick={() => void run(createEnrollmentToken)} disabled={busy}>
                {copy.settingsPage.generateEnrollmentToken}
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={copyAgentCommand} disabled={!enrollmentToken}>
                {copy.settingsPage.copyInstallCommand}
              </button>
              <a className={buttonClass("secondary")} href={installScriptUrl} target="_blank" rel="noopener noreferrer">
                {copy.settingsPage.openParameterizedLink}
              </a>
            </div>
            {enrollmentExpiresAt ? (
              <p className="mt-3 text-xs font-medium text-[var(--text-muted)]">
                {copy.settingsPage.tokenExpiresAt.replace("{value}", String(enrollmentExpiresAt))}
              </p>
            ) : null}
            <p className="mt-3 break-all text-xs font-medium text-[var(--text-muted)]">
              {copy.settingsPage.githubScriptSource.replace("{url}", String(githubScriptUrl))}
            </p>
          </div>
          <div>
            <p className="mb-2 text-xs font-medium text-[var(--text-muted)]">{copy.settingsPage.oneLineInstallCommand}</p>
            <pre className="min-h-40 overflow-auto whitespace-pre-wrap break-all rounded-xl border border-zinc-800 bg-zinc-950 p-3.5 font-mono text-xs text-emerald-400">
              {agentInstallCommand}
            </pre>
          </div>
        </div>
      </BrutalCard>
      {dialogs.element}
    </>
  );
}

function useRuntimeApiBaseUrl(): string {
  return useSyncExternalStore(subscribeRuntimeValue, getApiBaseUrl, () => HYDRATION_API_BASE_URL);
}

function readCachedLatestAgentVersion(): string | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem(GITHUB_RELEASE_CACHE_KEY);
    if (!raw) return null;
    const cached = JSON.parse(raw) as { tag?: unknown; cachedAt?: unknown };
    if (
      typeof cached.tag !== "string"
      || !cached.tag.trim()
      || typeof cached.cachedAt !== "number"
      || Date.now() - cached.cachedAt >= GITHUB_RELEASE_CACHE_TTL_MS
    ) {
      window.sessionStorage.removeItem(GITHUB_RELEASE_CACHE_KEY);
      return null;
    }
    return cached.tag.trim();
  } catch {
    return null;
  }
}

function cacheLatestAgentVersion(tag: string) {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.setItem(GITHUB_RELEASE_CACHE_KEY, JSON.stringify({
      tag,
      cachedAt: Date.now(),
    }));
  } catch {
    // Storage can be unavailable in private or restricted browser contexts.
  }
}

function scheduleWhenIdle(callback: () => void): () => void {
  if ("requestIdleCallback" in window) {
    const requestId = window.requestIdleCallback(callback, { timeout: 1500 });
    return () => window.cancelIdleCallback(requestId);
  }
  const timeoutId = setTimeout(callback, 250);
  return () => clearTimeout(timeoutId);
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

function shellQuote(value: string): string {
  return `'${value.replace(/[\u0000-\u001f\u007f]/g, "").replace(/'/g, "'\"'\"'")}'`;
}

function buildAgentInstallCommand({
  installScriptUrl,
}: {
  installScriptUrl: string;
}): string {
  return `curl -fsSL ${shellQuote(installScriptUrl)} | sudo bash`;
}
