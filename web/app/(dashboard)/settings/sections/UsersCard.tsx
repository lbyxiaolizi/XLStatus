"use client";

import { useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  StatusBadge,
  buttonClass,
  formatDate,
  inputClass,
  responseError,
} from "@/app/components/M7Primitives";
import { showToast } from "@/app/components/Toasts";
import { apiClient } from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { useBusy } from "@/app/lib/use-busy";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import { SectionFeedback, SettingsListSkeleton } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

interface UserAccount {
  id: string;
  username: string;
  role: string;
  created_at?: string;
  updated_at?: string;
}

export function UsersCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [busy, run] = useBusy();
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [users, setUsers] = useState<UserAccount[]>([]);
  const [usersLoading, setUsersLoading] = useState(true);
  const [newUser, setNewUser] = useState({ username: "", password: "", role: "member" });
  const [passwordEdits, setPasswordEdits] = useState<Record<string, string>>({});
  const cardRef = useLoadOnVisible(() => void loadUsers());

  async function loadUsers() {
    setUsersLoading(true);
    const response = await apiClient.listUsers(200, 0);
    setUsersLoading(false);
    if (response.success && response.data) {
      setUsers(((response.data.users as UserAccount[]) ?? []).filter((user) => user.id));
    } else {
      setError(responseError(response));
    }
  }

  async function createPanelUser() {
    if (newUser.password.length < 8) {
      setError(copy.settingsPage.userPasswordTooShort);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.createUser({
      username: newUser.username.trim(),
      password: newUser.password,
      role: newUser.role,
    }, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.userCreated);
      setNewUser({ username: "", password: "", role: "member" });
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function updateUserRole(user: UserAccount, role: string) {
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.updateUser(user.id, { role }, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.userRoleUpdated.replace("{username}", String(user.username)));
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function resetUserPassword(user: UserAccount) {
    const password = passwordEdits[user.id] ?? "";
    if (password.length < 8) {
      setError(copy.settingsPage.userPasswordResetTooShort);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.updateUser(user.id, { password }, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.userPasswordReset.replace("{username}", String(user.username)));
      setPasswordEdits((current) => ({ ...current, [user.id]: "" }));
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  async function deletePanelUser(user: UserAccount) {
    if (!(await dialogs.confirm({ message: copy.settingsPage.deleteUserConfirm.replace("{username}", String(user.username)), danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteUser(user.id, totpCode);
    if (response.success) {
      showToast(copy.settingsPage.userDeleted.replace("{username}", String(user.username)));
      await loadUsers();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold tracking-tight text-[var(--text-main)]">{copy.settingsPage.userManagement}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadUsers()} disabled={usersLoading}>
            {usersLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        <SectionFeedback error={error} />
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void run(createPanelUser);
          }}
          className="mb-5 grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_10rem_auto] md:items-end"
        >
          <Field label={copy.settingsPage.username}>
            <input className={inputClass} value={newUser.username} onChange={(e) => setNewUser({ ...newUser, username: e.target.value })} required />
          </Field>
          <Field label={copy.settingsPage.password}>
            <input className={inputClass} type="password" value={newUser.password} onChange={(e) => setNewUser({ ...newUser, password: e.target.value })} required />
          </Field>
          <Field label={copy.settingsPage.role}>
            <select className={inputClass} value={newUser.role} onChange={(e) => setNewUser({ ...newUser, role: e.target.value })}>
              <option value="member">member</option>
              <option value="admin">admin</option>
            </select>
          </Field>
          <button className={buttonClass("primary")} disabled={busy}>{copy.settingsPage.createUser}</button>
        </form>
        <div className="grid gap-3">
          {usersLoading && users.length === 0 ? (
            <SettingsListSkeleton />
          ) : users.length === 0 ? (
            <p className="text-sm font-medium text-[var(--text-muted)]">{copy.settingsPage.noUsers}</p>
          ) : (
            users.map((user) => (
              <div key={user.id} className="grid gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] p-3.5 xl:grid-cols-[minmax(0,1fr)_10rem_minmax(15rem,0.8fr)_auto] xl:items-center">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="break-words text-sm font-semibold text-[var(--text-main)]">{user.username}</span>
                    <StatusBadge tone={user.role === "admin" ? "yellow" : "gray"}>{user.role}</StatusBadge>
                  </div>
                  <div className="mt-1 break-all font-mono text-[11px] text-[var(--text-muted)]">{user.id}</div>
                  <div className="mt-1 text-xs text-[var(--text-muted)]">{copy.settingsPage.createdAt.replace("{date}", String(formatDate(user.created_at)))}</div>
                </div>
                <select className={inputClass} value={user.role} onChange={(e) => void run(() => updateUserRole(user, e.target.value))} disabled={busy}>
                  <option value="member">member</option>
                  <option value="admin">admin</option>
                </select>
                <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
                  <input
                    className={inputClass}
                    type="password"
                    placeholder={copy.settingsPage.newPassword}
                    value={passwordEdits[user.id] ?? ""}
                    onChange={(e) => setPasswordEdits((current) => ({ ...current, [user.id]: e.target.value }))}
                  />
                  <button type="button" className={buttonClass("secondary")} onClick={() => void run(() => resetUserPassword(user))} disabled={busy}>
                    {copy.settingsPage.reset}
                  </button>
                </div>
                <button type="button" className={buttonClass("danger")} onClick={() => void run(() => deletePanelUser(user))} disabled={busy}>
                  {copy.settingsPage.delete}
                </button>
              </div>
            ))
          )}
        </div>
      </BrutalCard>
      {dialogs.element}
    </div>
  );
}
