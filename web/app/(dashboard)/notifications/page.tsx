"use client";

import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import {
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  Modal,
  PageHeader,
  PageShell,
  StatusBadge,
  buttonClass,
  formatDate,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  textareaClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";

interface NotificationChannel {
  id: string;
  name: string;
  url: string;
  request_method: string;
  request_type: string;
  headers?: Record<string, string>;
  headers_json?: string | null;
  body_template?: string;
  verify_tls?: boolean;
  format_metric_units?: boolean;
  created_at?: string;
  updated_at?: string;
}

interface NotificationGroupMember {
  id: string;
  name: string;
  request_type?: string;
  url?: string;
}

interface NotificationGroup {
  id: string;
  name: string;
  members?: NotificationGroupMember[];
  created_at?: string;
  updated_at?: string;
}

interface NotificationProvider {
  id: string;
  name: string;
  request_method: string;
  request_type: string;
  body_template: string;
}

type ModalState =
  | { type: "notification"; item?: NotificationChannel }
  | { type: "group"; item?: NotificationGroup }
  | null;

const defaultBodyTemplate =
  '{"title":"{{title}}","message":"{{message}}","severity":"{{severity}}","timestamp":"{{timestamp}}"}';

function fallbackProviders(copy: Translations): NotificationProvider[] {
  return [
    {
      id: "generic-json",
      name: copy.notificationsPage.providerGenericJson,
      request_method: "POST",
      request_type: "json",
      body_template: defaultBodyTemplate,
    },
    {
      id: "generic-form",
      name: copy.notificationsPage.providerGenericForm,
      request_method: "POST",
      request_type: "form",
      body_template:
        "title={{title}}&message={{message}}&severity={{severity}}&timestamp={{timestamp}}",
    },
    {
      id: "custom",
      name: copy.notificationsPage.providerCustomBody,
      request_method: "POST",
      request_type: "custom",
      body_template: "{{message}}",
    },
    {
      id: "email-webhook",
      name: "Email Webhook",
      request_method: "POST",
      request_type: "json",
      body_template:
        '{"subject":"{{title}}","text":"{{message}}\\n\\n{{timestamp}}","severity":"{{severity}}"}',
    },
  ];
}

const emptyNotificationForm = {
  name: "",
  url: "",
  request_method: "POST",
  request_type: "json",
  headers_json: "",
  body_template: defaultBodyTemplate,
  verify_tls: true,
  format_metric_units: true,
};

const emptyGroupForm = {
  name: "",
};

const redactedSecret = "[redacted]";

export default function NotificationsPage() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const defaultProviders = useMemo(() => fallbackProviders(copy), [copy]);
  const [notifications, setNotifications] = useState<NotificationChannel[]>([]);
  const [groups, setGroups] = useState<NotificationGroup[]>([]);
  const [providers, setProviders] = useState<NotificationProvider[]>(defaultProviders);
  const [selectedMembers, setSelectedMembers] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState<ModalState>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
  const [notificationForm, setNotificationForm] = useState(emptyNotificationForm);
  const [notificationOriginals, setNotificationOriginals] = useState({
    url: "",
    headers_json: "",
    body_template: "",
  });
  const [groupForm, setGroupForm] = useState(emptyGroupForm);

  const load = useCallback(async () => {
    setError(null);
    const [notificationResponse, groupResponse, providerResponse] = await Promise.all([
      apiClient.listNotifications(200, 0),
      apiClient.listNotificationGroups(200, 0),
      apiClient.listNotificationProviders(),
    ]);
    let nextError: string | null = null;

    if (notificationResponse.success && notificationResponse.data) {
      setNotifications((notificationResponse.data.notifications as NotificationChannel[]) ?? []);
    } else {
      nextError = responseError(notificationResponse);
    }

    if (groupResponse.success && groupResponse.data) {
      setGroups((groupResponse.data.groups as NotificationGroup[]) ?? []);
    } else if (!nextError) {
      nextError = responseError(groupResponse);
    }

    if (providerResponse.success && providerResponse.data) {
      const nextProviders = (providerResponse.data.providers as NotificationProvider[]) ?? [];
      if (nextProviders.length) setProviders(nextProviders);
    }

    setError(nextError);
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void load();
  }, [load]);

  const notificationById = useMemo(
    () => new Map(notifications.map((item) => [item.id, item])),
    [notifications],
  );

  function openCreateNotification() {
    setNotificationForm(emptyNotificationForm);
    setNotificationOriginals({ url: "", headers_json: "", body_template: "" });
    setModal({ type: "notification" });
  }

  function openEditNotification(item: NotificationChannel) {
    const nextForm = {
      name: item.name || "",
      url: item.url || "",
      request_method: item.request_method || "POST",
      request_type: item.request_type || "json",
      headers_json: item.headers_json || formatHeaders(item.headers),
      body_template: item.body_template || "",
      verify_tls: item.verify_tls !== false,
      format_metric_units: item.format_metric_units !== false,
    };
    setNotificationForm({
      ...nextForm,
    });
    setNotificationOriginals({
      url: nextForm.url,
      headers_json: nextForm.headers_json,
      body_template: nextForm.body_template,
    });
    setModal({ type: "notification", item });
  }

  function openCreateGroup() {
    setGroupForm(emptyGroupForm);
    setModal({ type: "group" });
  }

  function openEditGroup(item: NotificationGroup) {
    setGroupForm({ name: item.name || "" });
    setModal({ type: "group", item });
  }

  function applyProvider(providerId: string) {
    const provider = providers.find((item) => item.id === providerId);
    if (!provider) return;
    setNotificationForm((form) => ({
      ...form,
      request_method: provider.request_method,
      request_type: provider.request_type,
      body_template: provider.body_template,
    }));
  }

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
      setError(copy.notificationsPage.totpInvalid);
      return null;
    }
    return trimmed;
  }

  async function submitNotification(event: FormEvent) {
    event.preventDefault();
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const editingItem = modal?.type === "notification" ? modal.item : undefined;
    const payload: JsonObject = {
      name: notificationForm.name.trim(),
      request_method: notificationForm.request_method,
      request_type: notificationForm.request_type,
      verify_tls: notificationForm.verify_tls,
      format_metric_units: notificationForm.format_metric_units,
    };
    if (!editingItem || notificationForm.url !== notificationOriginals.url || !isRedactedValue(notificationForm.url)) {
      payload.url = notificationForm.url.trim();
    }
    if (!editingItem || notificationForm.headers_json !== notificationOriginals.headers_json || !isRedactedValue(notificationForm.headers_json)) {
      payload.headers_json = notificationForm.headers_json.trim() || null;
    }
    if (!editingItem || notificationForm.body_template !== notificationOriginals.body_template || !isRedactedValue(notificationForm.body_template)) {
      payload.body_template = notificationForm.body_template;
    }
    const response =
      editingItem
        ? await apiClient.updateNotification(editingItem.id, payload, totpCode)
        : await apiClient.createNotification(payload, totpCode);

    if (response.success) {
      setNotice(modal?.type === "notification" && modal.item ? copy.notificationsPage.channelUpdated : copy.notificationsPage.channelCreated);
      setModal(null);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function submitGroup(event: FormEvent) {
    event.preventDefault();
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const payload: JsonObject = { name: groupForm.name.trim() };
    const response =
      modal?.type === "group" && modal.item
        ? await apiClient.updateNotificationGroup(modal.item.id, payload, totpCode)
        : await apiClient.createNotificationGroup(payload, totpCode);

    if (response.success) {
      setNotice(modal?.type === "group" && modal.item ? copy.notificationsPage.groupUpdated : copy.notificationsPage.groupCreated);
      setModal(null);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function testNotification(item: NotificationChannel) {
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.testNotification(item.id, totpCode);
    if (response.success) {
      setNotice(copy.notificationsPage.testSent.replace("{name}", String(item.name)));
    } else {
      setError(responseError(response));
    }
  }

  async function deleteNotification(item: NotificationChannel) {
    if (!(await dialogs.confirm({ message: copy.notificationsPage.deleteChannelConfirm.replace("{name}", String(item.name)), danger: true }))) return;
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteNotification(item.id, totpCode);
    if (response.success) {
      setNotice(copy.notificationsPage.channelDeleted);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteGroup(item: NotificationGroup) {
    if (!(await dialogs.confirm({ message: copy.notificationsPage.deleteGroupConfirm.replace("{name}", String(item.name)), danger: true }))) return;
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteNotificationGroup(item.id, totpCode);
    if (response.success) {
      setNotice(copy.notificationsPage.groupDeleted);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function addMember(group: NotificationGroup) {
    const notificationId = selectedMembers[group.id];
    if (!notificationId) return;
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.addNotificationGroupMember(group.id, notificationId, totpCode);
    if (response.success) {
      const notification = notificationById.get(notificationId);
      setNotice(copy.notificationsPage.memberAdded.replace("{name}", String(notification?.name || notificationId)));
      setSelectedMembers((current) => ({ ...current, [group.id]: "" }));
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function removeMember(group: NotificationGroup, member: NotificationGroupMember) {
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteNotificationGroupMember(group.id, member.id, totpCode);
    if (response.success) {
      setNotice(copy.notificationsPage.memberRemoved.replace("{name}", String(member.name)));
      await load();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow="Webhook"
          title={copy.notificationsPage.title}
          detail={copy.notificationsPage.detail}
          actions={
            <>
              <button type="button" className={buttonClass("secondary")} onClick={() => void load()}>
                {copy.notificationsPage.refresh}
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={openCreateGroup}>
                {copy.notificationsPage.addGroup}
              </button>
              <button type="button" className={buttonClass("primary")} onClick={openCreateNotification}>
                {copy.notificationsPage.addChannel}
              </button>
            </>
          }
        />

        <div className="mb-5 space-y-3">
          <InlineError message={error} />
          {notice ? <InlineNotice tone="green">{notice}</InlineNotice> : null}
        </div>

        <div className="grid gap-6 xl:grid-cols-[minmax(0,1.1fr)_minmax(340px,0.9fr)]">
          <section>
            <div className="mb-3 flex items-center justify-between gap-3">
              <h2 className="text-xl font-black uppercase">{copy.notificationsPage.channels}</h2>
              <StatusBadge tone="blue">{notifications.length}</StatusBadge>
            </div>
            {notifications.length === 0 ? (
              <EmptyState title={copy.notificationsPage.noChannels} />
            ) : (
              <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
                <table className="w-full min-w-[760px]">
                  <thead>
                    <tr>
                      <th className={thClass}>{copy.notificationsPage.colName}</th>
                      <th className={thClass}>{copy.notificationsPage.colType}</th>
                      <th className={thClass}>URL</th>
                      <th className={thClass}>{copy.notificationsPage.colTls}</th>
                      <th className={thClass}>{copy.notificationsPage.colUpdated}</th>
                      <th className={thClass}>{copy.notificationsPage.colActions}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {notifications.map((item) => (
                      <tr key={item.id}>
                        <td className={tdClass}>{item.name}</td>
                        <td className={tdClass}>
                          <div className="flex flex-wrap gap-2">
                            <StatusBadge tone="gray">{item.request_method || "POST"}</StatusBadge>
                            <StatusBadge tone="blue">{item.request_type || "json"}</StatusBadge>
                          </div>
                        </td>
                        <td className={`${tdClass} max-w-[280px] break-all text-xs`}>
                          {item.url}
                        </td>
                        <td className={tdClass}>
                          <StatusBadge tone={item.verify_tls === false ? "yellow" : "green"}>
                            {item.verify_tls === false ? copy.notificationsPage.tlsSkip : copy.notificationsPage.tlsVerify}
                          </StatusBadge>
                        </td>
                        <td className={tdClass}>{formatDate(item.updated_at || item.created_at)}</td>
                        <td className={`${tdClass} min-w-[220px]`}>
                          <div className="flex flex-wrap gap-2">
                            <button type="button" className={buttonClass("secondary")} onClick={() => void testNotification(item)}>
                              {copy.notificationsPage.test}
                            </button>
                            <button type="button" className={buttonClass("secondary")} onClick={() => openEditNotification(item)}>
                              {copy.notificationsPage.edit}
                            </button>
                            <button type="button" className={buttonClass("danger")} onClick={() => void deleteNotification(item)}>
                              {copy.notificationsPage.delete}
                            </button>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>

          <section>
            <div className="mb-3 flex items-center justify-between gap-3">
              <h2 className="text-xl font-black uppercase">{copy.notificationsPage.groups}</h2>
              <StatusBadge tone="pink">{groups.length}</StatusBadge>
            </div>
            {groups.length === 0 ? (
              <EmptyState title={copy.notificationsPage.noGroups} />
            ) : (
              <div className="grid gap-4">
                {groups.map((group) => {
                  const members = group.members ?? [];
                  const memberIds = new Set(members.map((member) => member.id));
                  const candidates = notifications.filter((item) => !memberIds.has(item.id));
                  return (
                    <article key={group.id} className="border-2 border-black bg-[var(--bg-card)] p-4 shadow-[var(--shadow-brutal)]">
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="min-w-0">
                          <h3 className="break-words text-lg font-black text-[var(--text-main)]">{group.name}</h3>
                          <p className="mt-1 text-xs font-black uppercase text-[var(--text-muted)]">
                            {members.length} {copy.notificationsPage.channelCountSuffix} / {formatDate(group.updated_at || group.created_at)}
                          </p>
                        </div>
                        <div className="flex flex-wrap gap-2">
                          <button type="button" className={buttonClass("secondary")} onClick={() => openEditGroup(group)}>
                            {copy.notificationsPage.edit}
                          </button>
                          <button type="button" className={buttonClass("danger")} onClick={() => void deleteGroup(group)}>
                            {copy.notificationsPage.delete}
                          </button>
                        </div>
                      </div>

                      <div className="mt-4 grid gap-2">
                        {members.length === 0 ? (
                          <div className="border-2 border-black bg-[var(--accent-bg)] px-3 py-3 text-sm font-black">
                            {copy.notificationsPage.noMembers}
                          </div>
                        ) : (
                          members.map((member) => (
                            <div key={member.id} className="grid gap-2 border-2 border-black px-3 py-2 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
                              <div className="min-w-0">
                                <p className="break-words text-sm font-black">{member.name}</p>
                                <p className="break-all text-xs font-bold text-[var(--text-muted)]">{member.url || member.id}</p>
                              </div>
                              <button type="button" className={buttonClass("secondary")} onClick={() => void removeMember(group, member)}>
                                {copy.notificationsPage.removeMember}
                              </button>
                            </div>
                          ))
                        )}
                      </div>

                      <div className="mt-4 grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
                        <select
                          className={selectClass}
                          value={selectedMembers[group.id] || ""}
                          onChange={(event) =>
                            setSelectedMembers((current) => ({
                              ...current,
                              [group.id]: event.target.value,
                            }))
                          }
                        >
                          <option value="">{copy.notificationsPage.selectChannel}</option>
                          {candidates.map((item) => (
                            <option key={item.id} value={item.id}>
                              {item.name}
                            </option>
                          ))}
                        </select>
                        <button
                          type="button"
                          className={buttonClass("good")}
                          disabled={!selectedMembers[group.id] || candidates.length === 0}
                          onClick={() => void addMember(group)}
                        >
                          {copy.notificationsPage.join}
                        </button>
                      </div>
                    </article>
                  );
                })}
              </div>
            )}
          </section>
        </div>

        {modal?.type === "notification" ? (
          <Modal title={modal.item ? copy.notificationsPage.editChannel : copy.notificationsPage.createChannel} onClose={() => setModal(null)}>
            <form onSubmit={submitNotification} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.notificationsPage.fieldName}>
                  <input className={inputClass} value={notificationForm.name} onChange={(event) => setNotificationForm((form) => ({ ...form, name: event.target.value }))} required />
                </Field>
                <Field label={copy.notificationsPage.fieldProvider}>
                  <select className={selectClass} onChange={(event) => applyProvider(event.target.value)} defaultValue="">
                    <option value="">{copy.notificationsPage.keepCurrentTemplate}</option>
                    {providers.map((provider) => (
                      <option key={provider.id} value={provider.id}>
                        {provider.name}
                      </option>
                    ))}
                  </select>
                </Field>
              </div>
              <Field label={copy.notificationsPage.fieldUrl}>
                <input className={inputClass} value={notificationForm.url} onChange={(event) => setNotificationForm((form) => ({ ...form, url: event.target.value }))} required />
              </Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.notificationsPage.fieldMethod}>
                  <select className={selectClass} value={notificationForm.request_method} onChange={(event) => setNotificationForm((form) => ({ ...form, request_method: event.target.value }))}>
                    <option value="POST">POST</option>
                    <option value="GET">GET</option>
                    <option value="PUT">PUT</option>
                    <option value="PATCH">PATCH</option>
                  </select>
                </Field>
                <Field label={copy.notificationsPage.fieldType}>
                  <select className={selectClass} value={notificationForm.request_type} onChange={(event) => setNotificationForm((form) => ({ ...form, request_type: event.target.value }))}>
                    <option value="json">json</option>
                    <option value="form">form</option>
                    <option value="custom">custom</option>
                  </select>
                </Field>
              </div>
              <Field label={copy.notificationsPage.fieldHeadersJson}>
                <textarea className={`${textareaClass} min-h-24`} value={notificationForm.headers_json} onChange={(event) => setNotificationForm((form) => ({ ...form, headers_json: event.target.value }))} spellCheck={false} />
              </Field>
              <Field label={copy.notificationsPage.fieldBodyTemplate}>
                <textarea className={`${textareaClass} min-h-32`} value={notificationForm.body_template} onChange={(event) => setNotificationForm((form) => ({ ...form, body_template: event.target.value }))} spellCheck={false} />
              </Field>
              <div className="grid gap-3 sm:grid-cols-2">
                <label className="flex items-center gap-3 border-2 border-black bg-[var(--bg-card)] px-3 py-3 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
                  <input type="checkbox" checked={notificationForm.verify_tls} onChange={(event) => setNotificationForm((form) => ({ ...form, verify_tls: event.target.checked }))} />
                  {copy.notificationsPage.verifyTlsCert}
                </label>
                <label className="flex items-center gap-3 border-2 border-black bg-[var(--bg-card)] px-3 py-3 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
                  <input type="checkbox" checked={notificationForm.format_metric_units} onChange={(event) => setNotificationForm((form) => ({ ...form, format_metric_units: event.target.checked }))} />
                  {copy.notificationsPage.formatMetricUnits}
                </label>
              </div>
              <button type="submit" className={buttonClass("primary")}>
                {copy.notificationsPage.saveChannel}
              </button>
            </form>
          </Modal>
        ) : null}

        {modal?.type === "group" ? (
          <Modal title={modal.item ? copy.notificationsPage.editGroup : copy.notificationsPage.createGroup} onClose={() => setModal(null)}>
            <form onSubmit={submitGroup} className="space-y-4">
              <Field label={copy.notificationsPage.fieldName}>
                <input className={inputClass} value={groupForm.name} onChange={(event) => setGroupForm({ name: event.target.value })} required />
              </Field>
              <button type="submit" className={buttonClass("primary")}>
                {copy.notificationsPage.saveGroup}
              </button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function formatHeaders(headers?: Record<string, string>): string {
  if (!headers || Object.keys(headers).length === 0) return "";
  return JSON.stringify(headers, null, 2);
}

function isRedactedValue(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  return trimmed === redactedSecret || trimmed.includes(redactedSecret);
}
