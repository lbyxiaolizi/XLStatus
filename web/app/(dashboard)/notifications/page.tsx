"use client";

import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import Navigation from "@/app/components/Navigation";
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

const fallbackProviders: NotificationProvider[] = [
  {
    id: "generic-json",
    name: "通用 JSON Webhook",
    request_method: "POST",
    request_type: "json",
    body_template: defaultBodyTemplate,
  },
  {
    id: "generic-form",
    name: "通用表单 Webhook",
    request_method: "POST",
    request_type: "form",
    body_template:
      "title={{title}}&message={{message}}&severity={{severity}}&timestamp={{timestamp}}",
  },
  {
    id: "custom",
    name: "自定义 Body",
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
  const [notifications, setNotifications] = useState<NotificationChannel[]>([]);
  const [groups, setGroups] = useState<NotificationGroup[]>([]);
  const [providers, setProviders] = useState<NotificationProvider[]>(fallbackProviders);
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
    const code = window.prompt("请输入 6 位 TOTP 验证码");
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError("请输入 6 位 TOTP 验证码。");
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
      setNotice(modal?.type === "notification" && modal.item ? "通知渠道已更新。" : "通知渠道已创建。");
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
      setNotice(modal?.type === "group" && modal.item ? "通知组已更新。" : "通知组已创建。");
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
      setNotice(`测试通知已发送：${item.name}`);
    } else {
      setError(responseError(response));
    }
  }

  async function deleteNotification(item: NotificationChannel) {
    if (!confirm(`确定删除通知渠道「${item.name}」？`)) return;
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteNotification(item.id, totpCode);
    if (response.success) {
      setNotice("通知渠道已删除。");
      await load();
    } else {
      setError(responseError(response));
    }
  }

  async function deleteGroup(item: NotificationGroup) {
    if (!confirm(`确定删除通知组「${item.name}」？`)) return;
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteNotificationGroup(item.id, totpCode);
    if (response.success) {
      setNotice("通知组已删除。");
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
      setNotice(`已加入通知组：${notification?.name || notificationId}`);
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
      setNotice(`已移出通知组：${member.name}`);
      await load();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="Webhook"
          title="通知"
          detail="通知渠道、模板和分组。"
          actions={
            <>
              <button type="button" className={buttonClass("secondary")} onClick={() => void load()}>
                刷新
              </button>
              <button type="button" className={buttonClass("secondary")} onClick={openCreateGroup}>
                新增通知组
              </button>
              <button type="button" className={buttonClass("primary")} onClick={openCreateNotification}>
                新增渠道
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
              <h2 className="text-xl font-black uppercase">渠道</h2>
              <StatusBadge tone="blue">{notifications.length}</StatusBadge>
            </div>
            {notifications.length === 0 ? (
              <EmptyState title="暂无通知渠道" />
            ) : (
              <div className="overflow-x-auto border-2 border-black bg-[var(--bg-card)] shadow-[var(--shadow-brutal)]">
                <table className="w-full min-w-[760px]">
                  <thead>
                    <tr>
                      <th className={thClass}>名称</th>
                      <th className={thClass}>类型</th>
                      <th className={thClass}>URL</th>
                      <th className={thClass}>TLS</th>
                      <th className={thClass}>更新</th>
                      <th className={thClass}>操作</th>
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
                            {item.verify_tls === false ? "跳过" : "校验"}
                          </StatusBadge>
                        </td>
                        <td className={tdClass}>{formatDate(item.updated_at || item.created_at)}</td>
                        <td className={`${tdClass} min-w-[220px]`}>
                          <div className="flex flex-wrap gap-2">
                            <button type="button" className={buttonClass("secondary")} onClick={() => void testNotification(item)}>
                              测试
                            </button>
                            <button type="button" className={buttonClass("secondary")} onClick={() => openEditNotification(item)}>
                              编辑
                            </button>
                            <button type="button" className={buttonClass("danger")} onClick={() => void deleteNotification(item)}>
                              删除
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
              <h2 className="text-xl font-black uppercase">通知组</h2>
              <StatusBadge tone="pink">{groups.length}</StatusBadge>
            </div>
            {groups.length === 0 ? (
              <EmptyState title="暂无通知组" />
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
                            {members.length} 个渠道 / {formatDate(group.updated_at || group.created_at)}
                          </p>
                        </div>
                        <div className="flex flex-wrap gap-2">
                          <button type="button" className={buttonClass("secondary")} onClick={() => openEditGroup(group)}>
                            编辑
                          </button>
                          <button type="button" className={buttonClass("danger")} onClick={() => void deleteGroup(group)}>
                            删除
                          </button>
                        </div>
                      </div>

                      <div className="mt-4 grid gap-2">
                        {members.length === 0 ? (
                          <div className="border-2 border-black bg-[var(--accent-bg)] px-3 py-3 text-sm font-black">
                            暂无成员
                          </div>
                        ) : (
                          members.map((member) => (
                            <div key={member.id} className="grid gap-2 border-2 border-black px-3 py-2 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
                              <div className="min-w-0">
                                <p className="break-words text-sm font-black">{member.name}</p>
                                <p className="break-all text-xs font-bold text-[var(--text-muted)]">{member.url || member.id}</p>
                              </div>
                              <button type="button" className={buttonClass("secondary")} onClick={() => void removeMember(group, member)}>
                                移出
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
                          <option value="">选择渠道</option>
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
                          加入
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
          <Modal title={modal.item ? "编辑通知渠道" : "新增通知渠道"} onClose={() => setModal(null)}>
            <form onSubmit={submitNotification} className="space-y-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="名称">
                  <input className={inputClass} value={notificationForm.name} onChange={(event) => setNotificationForm((form) => ({ ...form, name: event.target.value }))} required />
                </Field>
                <Field label="Provider">
                  <select className={selectClass} onChange={(event) => applyProvider(event.target.value)} defaultValue="">
                    <option value="">保持当前模板</option>
                    {providers.map((provider) => (
                      <option key={provider.id} value={provider.id}>
                        {provider.name}
                      </option>
                    ))}
                  </select>
                </Field>
              </div>
              <Field label="URL">
                <input className={inputClass} value={notificationForm.url} onChange={(event) => setNotificationForm((form) => ({ ...form, url: event.target.value }))} required />
              </Field>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Method">
                  <select className={selectClass} value={notificationForm.request_method} onChange={(event) => setNotificationForm((form) => ({ ...form, request_method: event.target.value }))}>
                    <option value="POST">POST</option>
                    <option value="GET">GET</option>
                    <option value="PUT">PUT</option>
                    <option value="PATCH">PATCH</option>
                  </select>
                </Field>
                <Field label="Type">
                  <select className={selectClass} value={notificationForm.request_type} onChange={(event) => setNotificationForm((form) => ({ ...form, request_type: event.target.value }))}>
                    <option value="json">json</option>
                    <option value="form">form</option>
                    <option value="custom">custom</option>
                  </select>
                </Field>
              </div>
              <Field label="Headers JSON">
                <textarea className={`${textareaClass} min-h-24`} value={notificationForm.headers_json} onChange={(event) => setNotificationForm((form) => ({ ...form, headers_json: event.target.value }))} spellCheck={false} />
              </Field>
              <Field label="Body Template">
                <textarea className={`${textareaClass} min-h-32`} value={notificationForm.body_template} onChange={(event) => setNotificationForm((form) => ({ ...form, body_template: event.target.value }))} spellCheck={false} />
              </Field>
              <div className="grid gap-3 sm:grid-cols-2">
                <label className="flex items-center gap-3 border-2 border-black bg-[var(--bg-card)] px-3 py-3 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
                  <input type="checkbox" checked={notificationForm.verify_tls} onChange={(event) => setNotificationForm((form) => ({ ...form, verify_tls: event.target.checked }))} />
                  TLS 证书校验
                </label>
                <label className="flex items-center gap-3 border-2 border-black bg-[var(--bg-card)] px-3 py-3 text-sm font-black shadow-[var(--shadow-brutal-sm)]">
                  <input type="checkbox" checked={notificationForm.format_metric_units} onChange={(event) => setNotificationForm((form) => ({ ...form, format_metric_units: event.target.checked }))} />
                  格式化指标单位
                </label>
              </div>
              <button type="submit" className={buttonClass("primary")}>
                保存渠道
              </button>
            </form>
          </Modal>
        ) : null}

        {modal?.type === "group" ? (
          <Modal title={modal.item ? "编辑通知组" : "新增通知组"} onClose={() => setModal(null)}>
            <form onSubmit={submitGroup} className="space-y-4">
              <Field label="名称">
                <input className={inputClass} value={groupForm.name} onChange={(event) => setGroupForm({ name: event.target.value })} required />
              </Field>
              <button type="submit" className={buttonClass("primary")}>
                保存通知组
              </button>
            </form>
          </Modal>
        ) : null}
      </PageShell>
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
