"use client";

import { useRouter } from "next/navigation";
import { useCallback, useDeferredValue, useEffect, useMemo, useState } from "react";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  KpiCard,
  PageHeader,
  PageShell,
  StatusBadge,
  TableSkeleton,
  buttonClass,
  clearStoredUser,
  compactId,
  formatDate,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { showToast } from "@/app/components/Toasts";
import { useBusy } from "@/app/lib/use-busy";
import { useI18n } from "@/lib/use-i18n";

type RbacRole = "admin" | "user" | "viewer";
type RoleFilter = "all" | RbacRole;

const ROLE_FILTERS: readonly RoleFilter[] = ["all", "admin", "user", "viewer"];

interface RawUserAccount {
  id: string;
  username: string;
  role: string;
  enabled?: boolean | undefined;
  disabled?: boolean | undefined;
  status?: string | undefined;
  created_at?: string | undefined;
  updated_at?: string | undefined;
}

interface UserAccount {
  id: string;
  username: string;
  backendRole: string;
  displayRole: RbacRole;
  disabled: boolean;
  created_at?: string | undefined;
  updated_at?: string | undefined;
}

interface SessionInfo {
  id: string;
  user_id: string;
  username: string;
  role: string;
  ip?: string | null | undefined;
  user_agent?: string | null | undefined;
  expires_at?: string | undefined;
  created_at?: string | undefined;
  is_current?: boolean | undefined;
}

interface UserLocalMeta {
  displayRole?: RbacRole | undefined;
  disabled?: boolean | undefined;
}

function normalizeDisplayRole(rawRole: string, override?: RbacRole): RbacRole {
  const normalized = rawRole.trim().toLowerCase();
  if (normalized === "admin") return "admin";
  if (normalized === "viewer") return "viewer";
  if (normalized === "user") return "user";
  if (override && override !== "admin") return override;
  return "user";
}

function roleBadgeTone(role: RbacRole): "yellow" | "blue" | "gray" {
  if (role === "admin") return "yellow";
  if (role === "user") return "blue";
  return "gray";
}

function toBackendRole(role: RbacRole): "admin" | "member" {
  return role === "admin" ? "admin" : "member";
}

export default function UsersPage() {
  const router = useRouter();
  const dialogs = useDialogs();
  const { locale, t: copy } = useI18n();
  const isEn = locale === "en-US";

  const [rawUsers, setRawUsers] = useState<RawUserAccount[]>([]);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [sessionsLoading, setSessionsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);

  // Local metadata for granular RBAC role (user vs viewer) & account status (active vs disabled)
  const [localMetaById, setLocalMetaById] = useState<Record<string, UserLocalMeta>>({});

  // Filter & Drawer state
  const [roleFilter, setRoleFilter] = useState<RoleFilter>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [newUser, setNewUser] = useState<{
    username: string;
    password: string;
    role: RbacRole;
    disabled: boolean;
  }>({
    username: "",
    password: "",
    role: "user",
    disabled: false,
  });

  // Master-detail inspector state
  const [selectedUserId, setSelectedUserId] = useState<string | null>(null);
  const [passwordResetValue, setPasswordResetValue] = useState("");

  const [formBusy, runForm] = useBusy();
  const [inspectorBusy, runInspector] = useBusy();

  const loadAll = useCallback(async () => {
    setLoading(true);
    setSessionsLoading(true);
    const [usersResp, sessionsResp] = await Promise.all([
      apiClient.listUsers(200, 0),
      apiClient.listSessions(200, 0),
    ]);
    setLoading(false);
    setSessionsLoading(false);

    if (usersResp.success && usersResp.data) {
      const list = ((usersResp.data.users as RawUserAccount[]) ?? []).filter((u) => Boolean(u.id));
      setRawUsers(list);
    } else {
      setError(responseError(usersResp));
    }

    if (sessionsResp.success && sessionsResp.data) {
      const list = ((sessionsResp.data.sessions as SessionInfo[]) ?? []).filter((s) =>
        Boolean(s.id),
      );
      setSessions(list);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void loadAll();
  }, [loadAll]);

  const users = useMemo<UserAccount[]>(() => {
    return rawUsers.map((raw) => {
      const meta = localMetaById[raw.id];
      const displayRole = normalizeDisplayRole(raw.role, meta?.displayRole);
      const isDisabledFromBackend =
        raw.disabled === true ||
        raw.enabled === false ||
        raw.status?.toLowerCase() === "disabled";
      const disabled = meta?.disabled ?? isDisabledFromBackend;
      return {
        id: raw.id,
        username: raw.username,
        backendRole: raw.role,
        displayRole,
        disabled,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
      };
    });
  }, [localMetaById, rawUsers]);

  const selectedUser = useMemo<UserAccount | null>(() => {
    if (!selectedUserId) return null;
    return users.find((u) => u.id === selectedUserId) ?? null;
  }, [selectedUserId, users]);

  const sessionsByUserId = useMemo(() => {
    const map = new Map<string, SessionInfo[]>();
    for (const session of sessions) {
      const existing = map.get(session.user_id) ?? [];
      existing.push(session);
      map.set(session.user_id, existing);
    }
    return map;
  }, [sessions]);

  const selectedUserSessions = useMemo<SessionInfo[]>(() => {
    if (!selectedUser) return [];
    return sessions.filter(
      (s) => s.user_id === selectedUser.id || s.username === selectedUser.username,
    );
  }, [selectedUser, sessions]);

  const kpiStats = useMemo(() => {
    const total = users.length;
    const admins = users.filter((u) => u.displayRole === "admin").length;
    const disabled = users.filter((u) => u.disabled).length;
    const active = total - disabled;
    return { total, admins, active, disabled };
  }, [users]);

  const deferredQuery = useDeferredValue(searchQuery);

  const filteredUsers = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase();
    return users.filter((user) => {
      if (roleFilter !== "all" && user.displayRole !== roleFilter) {
        return false;
      }
      if (!needle) return true;
      return [
        user.username,
        user.id,
        user.displayRole,
        user.backendRole,
        user.disabled ? "disabled" : "active",
      ].some((field) => field.toLowerCase().includes(needle));
    });
  }, [deferredQuery, roleFilter, users]);

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

  async function handleCreateUser() {
    setError(null);
    if (newUser.password.length < 8) {
      setError(copy.settingsPage.userPasswordTooShort);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createUser(
      {
        username: newUser.username.trim(),
        password: newUser.password,
        role: toBackendRole(newUser.role),
      },
      totpCode,
    );
    if (response.success) {
      const createdId = (response.data as { id?: string } | undefined)?.id;
      if (createdId) {
        setLocalMetaById((prev) => ({
          ...prev,
          [createdId]: { displayRole: newUser.role, disabled: newUser.disabled },
        }));
        setSelectedUserId(createdId);
      }
      showToast(copy.settingsPage.userCreated);
      setNewUser({ username: "", password: "", role: "user", disabled: false });
      setDrawerOpen(false);
      await loadAll();
    } else {
      setError(responseError(response));
    }
  }

  async function handleUpdateUserRole(user: UserAccount, nextRole: RbacRole) {
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.updateUser(
      user.id,
      { role: toBackendRole(nextRole) },
      totpCode,
    );
    if (response.success) {
      setLocalMetaById((prev) => ({
        ...prev,
        [user.id]: { ...prev[user.id], displayRole: nextRole },
      }));
      showToast(copy.settingsPage.userRoleUpdated.replace("{username}", String(user.username)));
      await loadAll();
    } else {
      setError(responseError(response));
    }
  }

  function handleToggleUserDisabled(user: UserAccount, nextDisabled: boolean) {
    setLocalMetaById((prev) => ({
      ...prev,
      [user.id]: {
        ...prev[user.id],
        displayRole: user.displayRole,
        disabled: nextDisabled,
      },
    }));
    showToast(
      nextDisabled
        ? isEn
          ? `Account ${user.username} marked as disabled.`
          : `账号 ${user.username} 已设为停用状态。`
        : isEn
          ? `Account ${user.username} marked as active.`
          : `账号 ${user.username} 已恢复为活跃状态。`,
    );
  }

  async function handleResetPassword(user: UserAccount) {
    setError(null);
    if (passwordResetValue.length < 8) {
      setError(copy.settingsPage.userPasswordResetTooShort);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.updateUser(
      user.id,
      { password: passwordResetValue },
      totpCode,
    );
    if (response.success) {
      showToast(copy.settingsPage.userPasswordReset.replace("{username}", String(user.username)));
      setPasswordResetValue("");
      await loadAll();
    } else {
      setError(responseError(response));
    }
  }

  async function handleDeleteUser(user: UserAccount) {
    if (
      !(await dialogs.confirm({
        message: copy.settingsPage.deleteUserConfirm.replace("{username}", String(user.username)),
        danger: true,
      }))
    ) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteUser(user.id, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.userDeleted.replace("{username}", String(user.username)));
      if (selectedUserId === user.id) {
        setSelectedUserId(null);
      }
      await loadAll();
    } else {
      setError(responseError(response));
    }
  }

  async function handleRevokeSession(session: SessionInfo) {
    const label = session.is_current
      ? copy.settingsPage.currentSessionLabel
      : copy.settingsPage.userSessionLabel.replace("{username}", String(session.username));
    if (
      !(await dialogs.confirm({
        message: copy.settingsPage.revokeSessionConfirm.replace("{label}", label),
        danger: true,
      }))
    ) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteSession(session.id, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.sessionRevoked);
      await loadAll();
      if (session.is_current) {
        clearStoredUser();
        router.replace("/login");
      }
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div>
      <PageShell wide>
        <PageHeader
          eyebrow={isEn ? "Identity & Access Control" : "身份与访问控制 (RBAC)"}
          title={copy.settingsPage.userManagement}
          detail={
            isEn
              ? "Manage operator accounts, RBAC roles, password credentials, and active login sessions."
              : "集中管理控制台账号、RBAC 角色权限、密码重置及活跃登录会话审计。"
          }
          actions={
            <>
              <button
                type="button"
                className={buttonClass("secondary")}
                disabled={loading}
                onClick={() => void loadAll()}
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
                  : "+ 创建用户"}
              </button>
            </>
          }
        />

        {/* Top KPI Strip */}
        <section className="mb-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4" aria-label="User KPIs">
          <KpiCard
            label={isEn ? "Total Users" : "用户总数"}
            value={kpiStats.total}
            hint={isEn ? "Provisioned console identities" : "已创建的控制台身份账号"}
            badge={<StatusBadge tone="blue">IAM</StatusBadge>}
          />
          <KpiCard
            label={isEn ? "Admins" : "管理员 (Admins)"}
            value={kpiStats.admins}
            hint={isEn ? "Full control-plane privileges" : "拥有全局控制面管理权限"}
            badge={<StatusBadge tone="yellow">admin</StatusBadge>}
          />
          <KpiCard
            label={isEn ? "Active Users" : "活跃用户"}
            value={kpiStats.active}
            hint={`${sessions.length} ${isEn ? "active sessions" : "个在线活跃会话"}`}
            badge={
              <StatusBadge tone={kpiStats.active > 0 ? "green" : "gray"}>
                {copy.common.enabled}
              </StatusBadge>
            }
          />
          <KpiCard
            label={isEn ? "Disabled Users" : "已停用用户"}
            value={kpiStats.disabled}
            hint={isEn ? "Suspended or restricted accounts" : "被暂停或限制访问的账号"}
            badge={
              <StatusBadge tone={kpiStats.disabled > 0 ? "red" : "gray"}>
                {copy.common.disabled}
              </StatusBadge>
            }
          />
        </section>

        {/* Filter Bar */}
        <BrutalCard className="mb-5 p-4">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="flex flex-1 flex-col gap-3 sm:flex-row sm:items-center">
              <div className="min-w-0 flex-1">
                <input
                  type="search"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className={`${inputClass} mt-0`}
                  placeholder={
                    isEn
                      ? "Search username, user ID, or role..."
                      : "搜索用户名、用户 UUID 或角色..."
                  }
                  aria-label={copy.common.search}
                />
              </div>

              {/* Role Filter Pills (all / admin / user / viewer) */}
              <div
                className="inline-flex flex-wrap items-center gap-1 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-1"
                role="group"
                aria-label="Role Filter"
              >
                {ROLE_FILTERS.map((role) => {
                  const active = roleFilter === role;
                  return (
                    <button
                      key={role}
                      type="button"
                      onClick={() => setRoleFilter(role)}
                      className={`rounded-lg px-2.5 py-1 font-mono text-xs font-medium transition ${
                        active
                          ? "bg-[var(--text-main)] text-[var(--bg-card)] shadow-[var(--shadow-xs)]"
                          : "text-[var(--text-muted)] hover:bg-[var(--bg-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {role}
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
              + 创建用户
            </button>
          </div>
        </BrutalCard>

        <div className="mb-5 space-y-3">
          <InlineError message={error} />
        </div>

        {/* Collapsible Create User Drawer */}
        {drawerOpen ? (
          <BrutalCard accent className="mb-6 border-[var(--border-strong)]">
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3">
              <div>
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {copy.settingsPage.createUser}
                </h2>
                <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                  {isEn
                    ? "Provision a new operator account and assign an RBAC role."
                    : "创建新的控制台身份账号并分配初始 RBAC 角色权限。"}
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
                void runForm(() => handleCreateUser());
              }}
              className="grid gap-4 sm:grid-cols-2 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_12rem_auto] lg:items-end"
            >
              <Field label={copy.settingsPage.username}>
                <input
                  className={inputClass}
                  value={newUser.username}
                  onChange={(e) => setNewUser((prev) => ({ ...prev, username: e.target.value }))}
                  placeholder="operator-01"
                  required
                />
              </Field>
              <Field label={copy.settingsPage.password}>
                <input
                  className={inputClass}
                  type="password"
                  value={newUser.password}
                  onChange={(e) => setNewUser((prev) => ({ ...prev, password: e.target.value }))}
                  placeholder={isEn ? "At least 8 characters" : "至少 8 位字符"}
                  required
                />
              </Field>
              <Field label={copy.settingsPage.role}>
                <select
                  className={selectClass}
                  value={newUser.role}
                  onChange={(e) =>
                    setNewUser((prev) => ({ ...prev, role: e.target.value as RbacRole }))
                  }
                >
                  <option value="admin">admin</option>
                  <option value="user">user</option>
                  <option value="viewer">viewer</option>
                </select>
              </Field>
              <div className="flex items-center gap-2">
                <button type="submit" disabled={formBusy} className={buttonClass("primary")}>
                  {copy.settingsPage.createUser}
                </button>
              </div>
            </form>
          </BrutalCard>
        ) : null}

        {/* Master-Detail Split View */}
        <div
          className={`grid gap-6 ${
            selectedUser
              ? "xl:grid-cols-[minmax(0,1.2fr)_minmax(360px,0.85fr)]"
              : "grid-cols-1"
          }`}
        >
          {/* Left Master Column: Users Table */}
          <div className="min-w-0">
            {loading && users.length === 0 ? (
              <TableSkeleton columns={5} rows={4} />
            ) : filteredUsers.length === 0 ? (
              <EmptyState
                title={users.length === 0 ? copy.settingsPage.noUsers : copy.common.noMatches}
                detail={
                  users.length === 0
                    ? isEn
                      ? "Click '+ 创建用户' to create your first user account."
                      : "点击「+ 创建用户」添加新的控制台用户。"
                    : isEn
                      ? "Try adjusting the role filter or search query."
                      : "请尝试切换角色筛选或清空搜索关键词。"
                }
              />
            ) : (
              <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]">
                <table className="w-full min-w-[42rem]">
                  <thead>
                    <tr>
                      <th className={thClass}>{copy.settingsPage.username}</th>
                      <th className={thClass}>{copy.settingsPage.role}</th>
                      <th className={thClass}>{isEn ? "Status" : "状态"}</th>
                      <th className={thClass}>{copy.settingsPage.activeSessions}</th>
                      <th className={thClass}>{isEn ? "Created" : "创建时间"}</th>
                      <th className={thClass}>{isEn ? "Actions" : "操作"}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-[var(--border-color)]">
                    {filteredUsers.map((user) => {
                      const isSelected = selectedUser?.id === user.id;
                      const userSessions = sessionsByUserId.get(user.id) ?? [];

                      return (
                        <tr
                          key={user.id}
                          onClick={() => {
                            setSelectedUserId(user.id);
                            setPasswordResetValue("");
                          }}
                          className={`cursor-pointer transition-colors ${
                            isSelected
                              ? "bg-[var(--bg-muted)]/75"
                              : "hover:bg-[var(--bg-subtle)]"
                          }`}
                        >
                          <td className={tdClass}>
                            <div className="flex items-center gap-2.5">
                              <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] font-mono text-xs font-bold text-[var(--text-main)] uppercase">
                                {user.username.slice(0, 2)}
                              </span>
                              <div className="min-w-0">
                                <div className="truncate font-semibold text-[var(--text-main)]">
                                  {user.username}
                                </div>
                                <div className="font-mono text-[11px] text-[var(--text-muted)]">
                                  {compactId(user.id)}
                                </div>
                              </div>
                            </div>
                          </td>

                          <td className={tdClass}>
                            <StatusBadge tone={roleBadgeTone(user.displayRole)}>
                              {user.displayRole}
                            </StatusBadge>
                          </td>

                          <td className={tdClass}>
                            <StatusBadge tone={user.disabled ? "red" : "green"}>
                              {user.disabled ? copy.common.disabled : copy.common.enabled}
                            </StatusBadge>
                          </td>

                          <td className={tdClass}>
                            <span className="inline-flex items-center rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 font-mono text-xs text-[var(--text-main)]">
                              {userSessions.length}
                            </span>
                          </td>

                          <td className={`${tdClass} font-mono text-xs text-[var(--text-muted)]`}>
                            {formatDate(user.created_at)}
                          </td>

                          <td className={tdClass} onClick={(e) => e.stopPropagation()}>
                            <div className="flex flex-wrap items-center gap-1.5">
                              <button
                                type="button"
                                className={buttonClass(isSelected ? "primary" : "secondary")}
                                onClick={() => {
                                  setSelectedUserId(user.id);
                                  setPasswordResetValue("");
                                }}
                              >
                                {isEn ? "Inspect" : "详情管理"}
                              </button>
                              <button
                                type="button"
                                className={buttonClass("danger")}
                                disabled={inspectorBusy}
                                onClick={() => void runInspector(() => handleDeleteUser(user))}
                              >
                                {copy.settingsPage.delete}
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
          </div>

          {/* Right Sticky Column: User Inspector & Session Manager */}
          {selectedUser ? (
            <aside className="xl:sticky xl:top-20 xl:self-start">
              <BrutalCard className="space-y-5 border-[var(--border-strong)]">
                {/* Inspector Header */}
                <div className="flex items-start justify-between gap-3 border-b border-[var(--border-color)] pb-4">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <h2 className="break-words text-lg font-semibold tracking-tight text-[var(--text-main)]">
                        {selectedUser.username}
                      </h2>
                      <StatusBadge tone={roleBadgeTone(selectedUser.displayRole)}>
                        {selectedUser.displayRole}
                      </StatusBadge>
                      <StatusBadge tone={selectedUser.disabled ? "red" : "green"}>
                        {selectedUser.disabled ? copy.common.disabled : copy.common.enabled}
                      </StatusBadge>
                    </div>
                    <p className="mt-1 break-all font-mono text-xs text-[var(--text-muted)]">
                      {selectedUser.id}
                    </p>
                  </div>
                  <button
                    type="button"
                    className={buttonClass("ghost")}
                    onClick={() => setSelectedUserId(null)}
                  >
                    {copy.common.close}
                  </button>
                </div>

                {/* Role & Status Editor */}
                <div className="space-y-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-4">
                  <h3 className="text-xs font-semibold tracking-wider text-[var(--text-muted)] uppercase">
                    {isEn ? "RBAC Role & Account Status" : "RBAC 角色与账号状态"}
                  </h3>
                  <div className="grid gap-3 sm:grid-cols-2">
                    <Field label={copy.settingsPage.role}>
                      <select
                        className={selectClass}
                        value={selectedUser.displayRole}
                        disabled={inspectorBusy}
                        onChange={(e) =>
                          void runInspector(() =>
                            handleUpdateUserRole(selectedUser, e.target.value as RbacRole),
                          )
                        }
                      >
                        <option value="admin">admin</option>
                        <option value="user">user</option>
                        <option value="viewer">viewer</option>
                      </select>
                    </Field>
                    <Field label={isEn ? "Account Status" : "账号状态"}>
                      <select
                        className={selectClass}
                        value={selectedUser.disabled ? "disabled" : "active"}
                        onChange={(e) =>
                          handleToggleUserDisabled(
                            selectedUser,
                            e.target.value === "disabled",
                          )
                        }
                      >
                        <option value="active">{copy.common.enabled}</option>
                        <option value="disabled">{copy.common.disabled}</option>
                      </select>
                    </Field>
                  </div>
                </div>

                {/* Password Reset */}
                <div className="space-y-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-4">
                  <h3 className="text-xs font-semibold tracking-wider text-[var(--text-muted)] uppercase">
                    {isEn ? "Credential Reset" : "重置登录密码"}
                  </h3>
                  <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
                    <input
                      type="password"
                      className={`${inputClass} mt-0`}
                      value={passwordResetValue}
                      onChange={(e) => setPasswordResetValue(e.target.value)}
                      placeholder={copy.settingsPage.newPassword}
                    />
                    <button
                      type="button"
                      className={buttonClass("secondary")}
                      disabled={inspectorBusy}
                      onClick={() => void runInspector(() => handleResetPassword(selectedUser))}
                    >
                      {copy.settingsPage.reset}
                    </button>
                  </div>
                </div>

                {/* Active Sessions Table */}
                <div className="space-y-3">
                  <div className="flex items-center justify-between gap-2">
                    <h3 className="text-xs font-semibold tracking-wider text-[var(--text-muted)] uppercase">
                      {copy.settingsPage.activeSessions} ({selectedUserSessions.length})
                    </h3>
                  </div>

                  {sessionsLoading && selectedUserSessions.length === 0 ? (
                    <TableSkeleton columns={3} rows={2} />
                  ) : selectedUserSessions.length === 0 ? (
                    <div className="rounded-xl border border-dashed border-[var(--border-color)] bg-[var(--bg-subtle)] px-4 py-6 text-center text-xs text-[var(--text-muted)]">
                      {copy.settingsPage.noActiveSessions}
                    </div>
                  ) : (
                    <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)]">
                      <table className="w-full text-xs">
                        <thead>
                          <tr>
                            <th className={thClass}>IP / UA</th>
                            <th className={thClass}>{isEn ? "Expires" : "过期时间"}</th>
                            <th className={thClass}>{isEn ? "Action" : "操作"}</th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-[var(--border-color)]">
                          {selectedUserSessions.map((session) => (
                            <tr key={session.id}>
                              <td className="px-3 py-2.5">
                                <div className="flex flex-wrap items-center gap-1.5">
                                  <span className="font-mono font-medium text-[var(--text-main)]">
                                    {session.ip || "IP N/A"}
                                  </span>
                                  {session.is_current ? (
                                    <StatusBadge tone="green">
                                      {copy.settingsPage.current}
                                    </StatusBadge>
                                  ) : null}
                                </div>
                                <div className="mt-0.5 max-w-[14rem] truncate text-[11px] text-[var(--text-muted)]">
                                  {session.user_agent || "UA N/A"}
                                </div>
                              </td>
                              <td className="px-3 py-2.5 font-mono text-[11px] text-[var(--text-muted)]">
                                {formatDate(session.expires_at)}
                              </td>
                              <td className="px-3 py-2.5">
                                <button
                                  type="button"
                                  className={buttonClass("danger")}
                                  disabled={inspectorBusy}
                                  onClick={() =>
                                    void runInspector(() => handleRevokeSession(session))
                                  }
                                >
                                  {copy.settingsPage.revoke}
                                </button>
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  )}
                </div>

                {/* Footer Danger Action */}
                <div className="flex items-center justify-between border-t border-[var(--border-color)] pt-3">
                  <span className="text-xs text-[var(--text-muted)]">
                    {copy.settingsPage.createdAt.replace(
                      "{date}",
                      String(formatDate(selectedUser.created_at)),
                    )}
                  </span>
                  <button
                    type="button"
                    className={buttonClass("danger")}
                    disabled={inspectorBusy}
                    onClick={() => void runInspector(() => handleDeleteUser(selectedUser))}
                  >
                    {copy.settingsPage.delete}
                  </button>
                </div>
              </BrutalCard>
            </aside>
          ) : null}
        </div>
      </PageShell>
      {dialogs.element}
    </div>
  );
}
