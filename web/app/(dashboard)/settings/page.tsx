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
import { apiClient } from "@/lib/api";

export default function SettingsPage() {
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState("server:read service:read task:* nat:* ddns:*");
  const [tokens, setTokens] = useState<unknown[]>([]);
  const [createdToken, setCreatedToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

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
