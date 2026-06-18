"use client";

import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  Modal,
  PageShell,
  StatusBadge,
  buttonClass,
  formatDate,
  inputClass,
  isAdmin,
  responseError,
  selectClass,
  useStoredUser,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject } from "@/lib/api";

interface PatInfo {
  id: string;
  name: string;
  scopes: string[];
  server_ids?: string[] | null;
  expires_at?: string | null;
  last_used_at?: string | null;
  created_at: string;
}

interface Server {
  id: string;
  name: string;
  status: string;
}

interface TokenForm {
  name: string;
  scopes: string[];
  server_ids: string[];
  expires_at: string;
}

interface MemberForm {
  username: string;
  password: string;
  role: "admin" | "member";
}

const allScopes = [
  "server:read",
  "server:exec",
  "service:read",
  "service:write",
  "service:delete",
  "alert:read",
  "alert:write",
  "task:read",
  "task:write",
  "task:delete",
  "task:exec",
  "ddns:read",
  "ddns:write",
  "ddns:delete",
  "nat:read",
  "nat:write",
  "nat:delete",
];

const blankToken: TokenForm = {
  name: "",
  scopes: ["server:read"],
  server_ids: [],
  expires_at: "",
};

const blankMember: MemberForm = {
  username: "",
  password: "",
  role: "member",
};

export default function SettingsPage() {
  const user = useStoredUser();
  const [tokens, setTokens] = useState<PatInfo[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [createdToken, setCreatedToken] = useState<string | null>(null);
  const [modal, setModal] = useState<"token" | "member" | null>(null);
  const [tokenForm, setTokenForm] = useState<TokenForm>(blankToken);
  const [memberForm, setMemberForm] = useState<MemberForm>(blankMember);
  const [formError, setFormError] = useState<string | null>(null);

  const loadSettings = useCallback(async () => {
    setError(null);
    try {
      setLoading(true);
      const [tokenResponse, serverResponse] = await Promise.all([
        apiClient.listPats(),
        apiClient.listServers(200, 0),
      ]);
      if (tokenResponse.success && tokenResponse.data) {
        setTokens((tokenResponse.data as PatInfo[]) ?? []);
      } else if (tokenResponse.status !== 403) {
        setError(responseError(tokenResponse));
      }
      if (serverResponse.success && serverResponse.data) {
        setServers((serverResponse.data.servers as Server[]) ?? []);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadSettings();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadSettings]);

  const defaultServerIds = useMemo(() => servers.map((server) => server.id), [servers]);

  function openToken() {
    setTokenForm({ ...blankToken, server_ids: [] });
    setCreatedToken(null);
    setFormError(null);
    setModal("token");
  }

  function openMember() {
    setMemberForm(blankMember);
    setFormError(null);
    setModal("member");
  }

  async function submitToken(event: FormEvent) {
    event.preventDefault();
    setFormError(null);
    if (!tokenForm.name.trim()) {
      setFormError("Token name is required");
      return;
    }
    if (tokenForm.scopes.length === 0) {
      setFormError("Select at least one scope");
      return;
    }

    setSaving(true);
    const expiresAt = tokenForm.expires_at
      ? new Date(tokenForm.expires_at).toISOString()
      : null;
    const response = await apiClient.createPat({
      name: tokenForm.name.trim(),
      scopes: tokenForm.scopes,
      server_ids: tokenForm.server_ids.length ? tokenForm.server_ids : null,
      expires_at: expiresAt,
    });
    setSaving(false);

    if (response.success && response.data) {
      setCreatedToken(response.data.token);
      setNotice("API token created.");
      await loadSettings();
    } else {
      setFormError(responseError(response));
    }
  }

  async function submitMember(event: FormEvent) {
    event.preventDefault();
    setFormError(null);
    if (!memberForm.username.trim()) {
      setFormError("Username is required");
      return;
    }
    if (memberForm.password.length < 8) {
      setFormError("Password must be at least 8 characters");
      return;
    }

    setSaving(true);
    const response = await apiClient.createUser(memberForm as unknown as JsonObject);
    setSaving(false);

    if (response.success) {
      setModal(null);
      setNotice("User created.");
    } else {
      setFormError(responseError(response));
    }
  }

  async function revokeToken(token: PatInfo) {
    if (!confirm(`Revoke API token "${token.name}"? Existing clients using it will fail immediately.`)) {
      return;
    }
    const response = await apiClient.revokePat(token.id);
    if (response.success) {
      setNotice("API token revoked.");
      await loadSettings();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div className="min-h-screen bg-gray-100">
      <Navigation />
      <PageShell>
        <div className="mb-6">
          <h1 className="text-2xl font-bold text-gray-900">Settings</h1>
          <p className="mt-1 text-sm text-gray-500">Account, API token, and member access controls.</p>
        </div>

        <div className="mb-4 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice>{notice}</InlineNotice> : null}
          {!isAdmin(user) ? <InlineNotice tone="yellow">Member sessions can view account context but admin-only creation routes may return permission denied.</InlineNotice> : null}
        </div>

        <div className="grid gap-6 lg:grid-cols-[360px_minmax(0,1fr)]">
          <div className="space-y-6">
            <section className="rounded-lg bg-white p-5 shadow">
              <h2 className="text-base font-semibold text-gray-900">Current User</h2>
              <div className="mt-4 space-y-3 text-sm">
                <div className="flex justify-between gap-3">
                  <span className="text-gray-500">Username</span>
                  <span className="font-medium text-gray-900">{user?.username || "Unknown"}</span>
                </div>
                <div className="flex justify-between gap-3">
                  <span className="text-gray-500">Role</span>
                  <span>{isAdmin(user) ? <StatusBadge tone="blue">Admin</StatusBadge> : <StatusBadge tone="gray">Member</StatusBadge>}</span>
                </div>
              </div>
            </section>

            <section className="rounded-lg bg-white p-5 shadow">
              <h2 className="text-base font-semibold text-gray-900">Members</h2>
              <p className="mt-2 text-sm text-gray-500">Create member or admin users when signed in as an admin.</p>
              <button type="button" onClick={openMember} className={`${buttonClass("primary")} mt-4 w-full`}>
                Add User
              </button>
            </section>
          </div>

          <section className="rounded-lg bg-white p-5 shadow">
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <h2 className="text-base font-semibold text-gray-900">API Tokens</h2>
                <p className="mt-1 text-sm text-gray-500">Personal access tokens with scopes and optional server allowlists.</p>
              </div>
              <button type="button" onClick={openToken} className={buttonClass("primary")}>
                Create Token
              </button>
            </div>

            {loading ? (
              <div className="mt-6 text-sm text-gray-600">Loading tokens...</div>
            ) : tokens.length === 0 ? (
              <div className="mt-6">
                <EmptyState title="No API tokens" />
              </div>
            ) : (
              <div className="mt-6 space-y-3">
                {tokens.map((token) => (
                  <div key={token.id} className="rounded border border-gray-200 p-4">
                    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                      <div>
                        <h3 className="font-medium text-gray-900">{token.name}</h3>
                        <p className="mt-1 text-xs text-gray-500">{token.id}</p>
                      </div>
                      <button type="button" onClick={() => void revokeToken(token)} className={buttonClass("danger")}>
                        Revoke
                      </button>
                    </div>
                    <div className="mt-3 flex flex-wrap gap-2">
                      {token.scopes.map((scope) => (
                        <StatusBadge key={scope} tone="gray">{scope}</StatusBadge>
                      ))}
                    </div>
                    <div className="mt-3 grid gap-2 text-sm text-gray-500 sm:grid-cols-3">
                      <span>Servers: {token.server_ids?.length ? token.server_ids.length : "All"}</span>
                      <span>Created: {formatDate(token.created_at)}</span>
                      <span>Expires: {formatDate(token.expires_at)}</span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </section>
        </div>
      </PageShell>

      {modal === "token" ? (
        <Modal title="Create API Token" onClose={() => setModal(null)}>
          <form onSubmit={submitToken} className="space-y-4">
            <InlineError message={formError} />
            {createdToken ? (
              <InlineNotice tone="green">
                <span className="block font-medium">Token created</span>
                <code className="mt-2 block overflow-x-auto rounded bg-white/70 p-2 text-xs">{createdToken}</code>
              </InlineNotice>
            ) : null}

            <Field label="Name">
              <input value={tokenForm.name} onChange={(event) => setTokenForm((prev) => ({ ...prev, name: event.target.value }))} className={inputClass} />
            </Field>

            <Field label="Scopes">
              <select
                multiple
                value={tokenForm.scopes}
                onChange={(event) =>
                  setTokenForm((prev) => ({
                    ...prev,
                    scopes: Array.from(event.target.selectedOptions).map((option) => option.value),
                  }))
                }
                className={`${selectClass} min-h-44`}
              >
                {allScopes.map((scope) => (
                  <option key={scope} value={scope}>
                    {scope}
                  </option>
                ))}
              </select>
            </Field>

            <Field label="Server Allowlist">
              <select
                multiple
                value={tokenForm.server_ids}
                onChange={(event) =>
                  setTokenForm((prev) => ({
                    ...prev,
                    server_ids: Array.from(event.target.selectedOptions).map((option) => option.value),
                  }))
                }
                className={`${selectClass} min-h-32`}
              >
                {defaultServerIds.length === 0 ? <option disabled>No servers available</option> : null}
                {servers.map((server) => (
                  <option key={server.id} value={server.id}>
                    {server.name} ({server.status})
                  </option>
                ))}
              </select>
            </Field>

            <Field label="Expires At">
              <input type="datetime-local" value={tokenForm.expires_at} onChange={(event) => setTokenForm((prev) => ({ ...prev, expires_at: event.target.value }))} className={inputClass} />
            </Field>

            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setModal(null)} className={buttonClass()}>
                Close
              </button>
              <button type="submit" disabled={saving || Boolean(createdToken)} className={buttonClass("primary")}>
                {saving ? "Creating..." : "Create Token"}
              </button>
            </div>
          </form>
        </Modal>
      ) : null}

      {modal === "member" ? (
        <Modal title="Add User" onClose={() => setModal(null)}>
          <form onSubmit={submitMember} className="space-y-4">
            <InlineError message={formError} />
            <Field label="Username">
              <input value={memberForm.username} onChange={(event) => setMemberForm((prev) => ({ ...prev, username: event.target.value }))} className={inputClass} />
            </Field>
            <Field label="Password">
              <input type="password" value={memberForm.password} onChange={(event) => setMemberForm((prev) => ({ ...prev, password: event.target.value }))} className={inputClass} />
            </Field>
            <Field label="Role">
              <select value={memberForm.role} onChange={(event) => setMemberForm((prev) => ({ ...prev, role: event.target.value as "admin" | "member" }))} className={selectClass}>
                <option value="member">Member</option>
                <option value="admin">Admin</option>
              </select>
            </Field>
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setModal(null)} className={buttonClass()}>
                Cancel
              </button>
              <button type="submit" disabled={saving} className={buttonClass("primary")}>
                {saving ? "Creating..." : "Create User"}
              </button>
            </div>
          </form>
        </Modal>
      ) : null}
    </div>
  );
}
