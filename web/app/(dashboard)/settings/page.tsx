"use client";

import { FormEvent, useEffect, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  BrutalCard,
  Field,
  InlineError,
  InlineNotice,
  PageHeader,
  PageShell,
  buttonClass,
  inputClass,
  responseError,
  textareaClass,
} from "@/app/components/M7Primitives";
import { apiClient, getApiBaseUrl } from "@/lib/api";

export default function SettingsPage() {
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState("server:read service:read task:* nat:* ddns:*");
  const [tokens, setTokens] = useState<unknown[]>([]);
  const [createdToken, setCreatedToken] = useState("");
  const [agentServerUrl, setAgentServerUrl] = useState(() => getApiBaseUrl());
  const [agentGrpcUrl, setAgentGrpcUrl] = useState(() => defaultGrpcUrl(getApiBaseUrl()));
  const [agentName, setAgentName] = useState("$(hostname)");
  const [agentVersion, setAgentVersion] = useState("v1.0.0");
  const [enrollmentHours, setEnrollmentHours] = useState("24");
  const [enrollmentToken, setEnrollmentToken] = useState("");
  const [enrollmentExpiresAt, setEnrollmentExpiresAt] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const installScriptUrl = apiClient.getAgentInstallScriptUrl({
    server_url: agentServerUrl,
    grpc_server: agentGrpcUrl,
    enrollment_token: enrollmentToken || "xle_...",
    agent_name: agentName,
    version: agentVersion,
  });
  const githubScriptUrl = `https://github.com/lbyxiaolizi/XLStatus/releases/download/${encodeURIComponent(agentVersion)}/install-agent.sh`;
  const agentInstallCommand = buildAgentInstallCommand({
    installScriptUrl,
  });

  useEffect(() => {
    void loadTokens();
  }, []);

  async function loadTokens() {
    const response = await apiClient.listPats();
    if (response.success && response.data) {
      setTokens(response.data);
    } else {
      setError(responseError(response));
    }
  }

  async function createToken(event: FormEvent) {
    event.preventDefault();
    const response = await apiClient.createPat({
      name,
      scopes: scopes.split(/\s+/).filter(Boolean),
    });
    if (response.success && response.data) {
      setCreatedToken(response.data.token);
      setNotice("个人访问令牌已创建。");
      setName("");
      await loadTokens();
    } else {
      setError(responseError(response));
    }
  }

  async function createEnrollmentToken() {
    const expiresInHours = Number.parseInt(enrollmentHours, 10);
    const response = await apiClient.createEnrollmentToken(
      Number.isFinite(expiresInHours) && expiresInHours > 0 ? expiresInHours : 24,
    );
    if (response.success && response.data) {
      setEnrollmentToken(response.data.token);
      setEnrollmentExpiresAt(response.data.expires_at);
      setNotice("Agent 安装令牌已创建。");
    } else {
      setError(responseError(response));
    }
  }

  async function copyAgentCommand() {
    try {
      await navigator.clipboard.writeText(agentInstallCommand);
      setNotice("Agent 安装命令已复制。");
    } catch {
      setError("无法写入剪贴板，请手动复制命令。");
    }
  }

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="控制面"
          title="设置"
          detail="个人访问令牌和本地管理员辅助工具。"
        />
        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        <BrutalCard accent className="mb-6">
          <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
            <div>
              <h2 className="mb-4 text-xl font-black uppercase">Agent 安装</h2>
              <div className="grid gap-4 md:grid-cols-2">
                <Field label="Server URL">
                  <input className={inputClass} value={agentServerUrl} onChange={(e) => setAgentServerUrl(e.target.value)} />
                </Field>
                <Field label="gRPC URL">
                  <input className={inputClass} value={agentGrpcUrl} onChange={(e) => setAgentGrpcUrl(e.target.value)} />
                </Field>
                <Field label="Release 版本">
                  <input className={inputClass} value={agentVersion} onChange={(e) => setAgentVersion(e.target.value)} />
                </Field>
                <Field label="Agent 名称">
                  <input className={inputClass} value={agentName} onChange={(e) => setAgentName(e.target.value)} />
                </Field>
                <Field label="令牌有效期（小时）">
                  <input className={inputClass} type="number" min="1" value={enrollmentHours} onChange={(e) => setEnrollmentHours(e.target.value)} />
                </Field>
              </div>
              <div className="mt-4 flex flex-wrap gap-2">
                <button type="button" className={buttonClass("primary")} onClick={createEnrollmentToken}>
                  生成安装令牌
                </button>
                <button type="button" className={buttonClass("secondary")} onClick={copyAgentCommand} disabled={!enrollmentToken}>
                  复制安装命令
                </button>
                <a className={buttonClass("secondary")} href={installScriptUrl} target="_blank" rel="noreferrer">
                  打开带参链接
                </a>
              </div>
              {enrollmentExpiresAt ? (
                <p className="mt-3 text-xs font-black uppercase text-[var(--text-muted)]">
                  令牌过期时间：{enrollmentExpiresAt}
                </p>
              ) : null}
              <p className="mt-3 break-all text-xs font-bold text-[var(--text-muted)]">
                GitHub 脚本源：{githubScriptUrl}
              </p>
            </div>
            <div>
              <p className="mb-2 text-xs font-black uppercase text-[var(--text-muted)]">带参数一键安装命令</p>
              <pre className="min-h-40 overflow-auto whitespace-pre-wrap break-all border-2 border-black bg-black p-3 font-mono text-xs text-green-300 shadow-[var(--shadow-brutal-sm)]">
                {agentInstallCommand}
              </pre>
            </div>
          </div>
        </BrutalCard>

        <div className="grid gap-6 lg:grid-cols-2">
          <BrutalCard accent>
            <h2 className="mb-4 text-xl font-black uppercase">创建 PAT</h2>
            <form onSubmit={createToken} className="space-y-4">
              <Field label="名称"><input className={inputClass} value={name} onChange={(e) => setName(e.target.value)} required /></Field>
              <Field label="Scope"><textarea className={`${textareaClass} min-h-24`} value={scopes} onChange={(e) => setScopes(e.target.value)} /></Field>
              <button className={buttonClass("primary")}>创建令牌</button>
            </form>
            {createdToken ? (
              <div className="mt-5 border-2 border-black bg-black p-3 font-mono text-xs text-green-300">
                {createdToken}
              </div>
            ) : null}
          </BrutalCard>

          <BrutalCard>
            <h2 className="mb-4 text-xl font-black uppercase">已有令牌</h2>
            <div className="grid gap-3">
              {tokens.length === 0 ? (
                <p className="text-sm font-bold text-[var(--text-muted)]">暂无令牌。</p>
              ) : (
                tokens.map((token, index) => (
                  <div key={index} className="border-2 border-black bg-[var(--accent-bg)] p-3 text-sm font-bold">
                    <pre className="overflow-auto whitespace-pre-wrap">{JSON.stringify(token, null, 2)}</pre>
                  </div>
                ))
              )}
            </div>
          </BrutalCard>
        </div>
      </PageShell>
    </div>
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

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\"'\"'")}'`;
}

function buildAgentInstallCommand({
  installScriptUrl,
}: {
  installScriptUrl: string;
}): string {
  return `curl -fsSL ${shellQuote(installScriptUrl)} | sudo bash`;
}
