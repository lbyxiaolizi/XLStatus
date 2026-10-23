"use client";

import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  PageHeader,
  PageShell,
  ProgressBar,
  StatusBadge,
  asRecord,
  asString,
  buttonClass,
  formatDate,
  formatPercent,
  inputClass,
  responseError,
  selectClass,
  tdClass,
  textareaClass,
  thClass,
} from "@/app/components/M7Primitives";
import { apiClient, type JsonObject, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { showToast } from "@/app/components/Toasts";
import { useBusy } from "@/app/lib/use-busy";
import { useI18n } from "@/lib/use-i18n";
import type { Translations } from "@/lib/i18n";

type WorkspaceTab = "overview" | "events" | "rules" | "channels";
type EventStatusFilter = "all" | "firing" | "resolved";
type EventSeverityFilter = "all" | "critical" | "warning" | "info";

interface AlertRule {
  id: string;
  name: string;
  trigger?: string | undefined;
  conditions?: JsonObject[] | undefined;
  enabled?: boolean | undefined;
  notification_group_id?: string | null | undefined;
  failure_task_ids?: string[] | undefined;
  recovery_task_ids?: string[] | undefined;
  created_at?: string | undefined;
}

interface AlertEvent {
  id?: string | undefined;
  kind?: string | undefined;
  rule_id?: string | undefined;
  rule_name?: string | undefined;
  message?: string | undefined;
  status?: string | undefined;
  severity?: string | undefined;
  payload?: unknown;
  fired_at?: string | undefined;
  created_at?: string | undefined;
}

interface NotificationChannel {
  id: string;
  name: string;
  url: string;
  request_method: string;
  request_type: string;
  headers?: Record<string, string> | undefined;
  headers_json?: string | null | undefined;
  body_template?: string | undefined;
  verify_tls?: boolean | undefined;
  format_metric_units?: boolean | undefined;
  created_at?: string | undefined;
  updated_at?: string | undefined;
}

interface NotificationGroup {
  id: string;
  name: string;
}

interface NotificationProvider {
  id: string;
  name: string;
  request_method: string;
  request_type: string;
  body_template: string;
}

interface RuleFormState {
  name: string;
  trigger: string;
  condition_type: string;
  agent_id: string;
  service_id: string;
  resource: string;
  operator: string;
  threshold: string;
  duration_seconds: string;
  offline_seconds: string;
  consecutive_failures: string;
  max_latency_ms: string;
  cert_days_before: string;
  server_expiry_days_before: string;
  traffic_quota_percent: string;
  traffic_quota_direction: string;
  notification_group_id: string;
  failure_task_ids: string;
  recovery_task_ids: string;
  enabled: boolean;
}

const defaultBodyTemplate =
  '{"title":"{{title}}","message":"{{message}}","severity":"{{severity}}","timestamp":"{{timestamp}}"}';

const emptyRuleForm: RuleFormState = {
  name: "",
  trigger: "once",
  condition_type: "server_resource",
  agent_id: "",
  service_id: "",
  resource: "cpu",
  operator: "gt",
  threshold: "90",
  duration_seconds: "60",
  offline_seconds: "120",
  consecutive_failures: "3",
  max_latency_ms: "1000",
  cert_days_before: "7",
  server_expiry_days_before: "7",
  traffic_quota_percent: "80",
  traffic_quota_direction: "total",
  notification_group_id: "",
  failure_task_ids: "",
  recovery_task_ids: "",
  enabled: true,
};

const emptyChannelForm = {
  name: "",
  url: "",
  request_method: "POST",
  request_type: "json",
  headers_json: "",
  body_template: defaultBodyTemplate,
  verify_tls: true,
  format_metric_units: true,
};

const redactedSecret = "[redacted]";

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
  ];
}

// Semantic wrappers over apiClient notification channel endpoints
async function listNotificationChannels(limit = 100, offset = 0) {
  return apiClient.listNotifications(limit, offset);
}
async function createNotificationChannel(payload: JsonObject, totpCode?: string) {
  return apiClient.createNotification(payload, totpCode);
}
async function updateNotificationChannel(id: string, payload: JsonObject, totpCode?: string) {
  return apiClient.updateNotification(id, payload, totpCode);
}
async function deleteNotificationChannel(id: string, totpCode?: string) {
  return apiClient.deleteNotification(id, totpCode);
}
async function testNotificationChannel(id: string, totpCode?: string) {
  return apiClient.testNotification(id, totpCode);
}

export default function AlertsPage() {
  const dialogs = useDialogs();
  const { locale, t: copy } = useI18n();
  const isZh = locale === "zh-CN";
  const defaultProviders = useMemo(() => fallbackProviders(copy), [copy]);

  const [activeTab, setActiveTab] = useState<WorkspaceTab>("overview");
  const [rules, setRules] = useState<AlertRule[]>([]);
  const [events, setEvents] = useState<AlertEvent[]>([]);
  const [channels, setChannels] = useState<NotificationChannel[]>([]);
  const [groups, setGroups] = useState<NotificationGroup[]>([]);
  const [providers, setProviders] = useState<NotificationProvider[]>(defaultProviders);
  const [error, setError] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);

  // Collapsible Creation / Edit Drawers
  const [showRuleForm, setShowRuleForm] = useState(false);
  const [editingRule, setEditingRule] = useState<AlertRule | null>(null);
  const [showChannelForm, setShowChannelForm] = useState(false);
  const [editingChannel, setEditingChannel] = useState<NotificationChannel | null>(null);

  // Event stream filters
  const [eventQuery, setEventQuery] = useState("");
  const [eventStatusFilter, setEventStatusFilter] = useState<EventStatusFilter>("all");
  const [eventSeverityFilter, setEventSeverityFilter] = useState<EventSeverityFilter>("all");

  // Busy guards
  const [formBusy, runForm] = useBusy();
  const [rowBusy, runRow] = useBusy();
  const [channelBusy, runChannel] = useBusy();

  const [form, setForm] = useState<RuleFormState>(emptyRuleForm);
  const [channelForm, setChannelForm] = useState(emptyChannelForm);
  const [channelOriginals, setChannelOriginals] = useState({
    url: "",
    headers_json: "",
    body_template: "",
  });

  const loadRequestIdRef = useRef(0);

  const load = useCallback(async (force = false) => {
    const requestId = loadRequestIdRef.current + 1;
    loadRequestIdRef.current = requestId;
    const [
      rulesResponse,
      eventsResponse,
      channelsResponse,
      groupsResponse,
      providersResponse,
    ] = await Promise.all([
      apiClient.listAlertRules(100, 0),
      apiClient.listAlertEvents(80, { bypassClientCache: force }),
      listNotificationChannels(100, 0),
      apiClient.listNotificationGroups(100, 0, { bypassClientCache: force }),
      apiClient.listNotificationProviders(),
    ]);
    if (requestId !== loadRequestIdRef.current) return;

    if (rulesResponse.success && rulesResponse.data) {
      setRules((rulesResponse.data["rules"] as AlertRule[]) ?? []);
    } else {
      setError(responseError(rulesResponse));
    }
    if (eventsResponse.success && eventsResponse.data) {
      setEvents((eventsResponse.data["events"] as AlertEvent[]) ?? []);
    }
    if (channelsResponse.success && channelsResponse.data) {
      setChannels((channelsResponse.data["notifications"] as NotificationChannel[]) ?? []);
    }
    if (groupsResponse.success && groupsResponse.data) {
      setGroups((groupsResponse.data["groups"] as NotificationGroup[]) ?? []);
    }
    if (providersResponse.success && providersResponse.data) {
      const nextProviders = (providersResponse.data["providers"] as NotificationProvider[]) ?? [];
      if (nextProviders.length) setProviders(nextProviders);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount is the standard client data-load pattern
    void load();
    return () => {
      loadRequestIdRef.current += 1;
    };
  }, [load]);

  // KPI computations
  const kpi = useMemo(() => {
    const activeIncidents = events.filter((e) => extractEventStatus(e) !== "resolved").length;
    const enabledRules = rules.filter((r) => r.enabled !== false).length;
    const ruleRatio = rules.length > 0 ? (enabledRules / rules.length) * 100 : 100;
    const tlsVerifiedChannels = channels.filter((c) => c.verify_tls !== false).length;
    return {
      activeIncidents,
      totalEvents: events.length,
      enabledRules,
      totalRules: rules.length,
      ruleRatio,
      totalChannels: channels.length,
      totalGroups: groups.length,
      tlsVerifiedChannels,
    };
  }, [events, rules, channels, groups]);

  const deferredEventQuery = useDeferredValue(eventQuery);
  const filteredEvents = useMemo(() => {
    const needle = deferredEventQuery.trim().toLowerCase();
    return events.filter((event) => {
      const status = extractEventStatus(event);
      const severity = extractEventSeverity(event);
      if (eventStatusFilter === "firing" && status === "resolved") return false;
      if (eventStatusFilter === "resolved" && status !== "resolved") return false;
      if (eventSeverityFilter !== "all" && severity !== eventSeverityFilter) return false;
      if (!needle) return true;
      const title = extractEventTitle(copy, event);
      const body = formatPayload(copy, event.payload ?? event.message);
      return [title, body, status, severity]
        .filter(Boolean)
        .some((v) => String(v).toLowerCase().includes(needle));
    });
  }, [deferredEventQuery, eventStatusFilter, eventSeverityFilter, events, copy]);

  function toggleRuleDrawer() {
    if (showRuleForm && !editingRule) {
      setShowRuleForm(false);
      return;
    }
    setEditingRule(null);
    setForm(emptyRuleForm);
    setShowChannelForm(false);
    setShowRuleForm(true);
  }

  function openEditRule(rule: AlertRule) {
    setEditingRule(rule);
    setForm(ruleToFormState(rule));
    setShowChannelForm(false);
    setShowRuleForm(true);
  }

  function toggleChannelDrawer() {
    if (showChannelForm && !editingChannel) {
      setShowChannelForm(false);
      return;
    }
    setEditingChannel(null);
    setChannelForm(emptyChannelForm);
    setChannelOriginals({ url: "", headers_json: "", body_template: "" });
    setShowRuleForm(false);
    setShowChannelForm(true);
  }

  function openEditChannel(channel: NotificationChannel) {
    const nextForm = {
      name: channel.name || "",
      url: channel.url || "",
      request_method: channel.request_method || "POST",
      request_type: channel.request_type || "json",
      headers_json: channel.headers_json || formatHeaders(channel.headers),
      body_template: channel.body_template || defaultBodyTemplate,
      verify_tls: channel.verify_tls !== false,
      format_metric_units: channel.format_metric_units !== false,
    };
    setEditingChannel(channel);
    setChannelForm(nextForm);
    setChannelOriginals({
      url: nextForm.url,
      headers_json: nextForm.headers_json,
      body_template: nextForm.body_template,
    });
    setShowRuleForm(false);
    setShowChannelForm(true);
  }

  function applyProvider(providerId: string) {
    const provider = providers.find((item) => item.id === providerId);
    if (!provider) return;
    setChannelForm((prev) => ({
      ...prev,
      request_method: provider.request_method,
      request_type: provider.request_type,
      body_template: provider.body_template,
    }));
  }

  async function submitRule() {
    setError(null);
    const payload: JsonObject = {
      name: form.name.trim(),
      trigger: form.trigger,
      enabled: form.enabled,
      conditions: [buildCondition(form)],
      notification_group_id: form.notification_group_id.trim() || null,
      failure_task_ids: splitIds(form.failure_task_ids),
      recovery_task_ids: splitIds(form.recovery_task_ids),
    };
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = editingRule
      ? await apiClient.updateAlertRule(editingRule.id, payload, totpCode)
      : await apiClient.createAlertRule(payload, totpCode);
    if (response.success) {
      showToast(copy.alertsPage.ruleCreated);
      setShowRuleForm(false);
      setEditingRule(null);
      await load(true);
    } else {
      setError(responseError(response));
    }
  }

  async function toggleRuleEnabled(rule: AlertRule) {
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const nextEnabled = rule.enabled === false;
    const payload: JsonObject = {
      name: rule.name,
      trigger: rule.trigger ?? "once",
      enabled: nextEnabled,
      ...(rule.conditions ? { conditions: rule.conditions } : {}),
      ...(rule.notification_group_id !== undefined
        ? { notification_group_id: rule.notification_group_id }
        : {}),
      ...(rule.failure_task_ids ? { failure_task_ids: rule.failure_task_ids } : {}),
      ...(rule.recovery_task_ids ? { recovery_task_ids: rule.recovery_task_ids } : {}),
    };
    const response = await apiClient.updateAlertRule(rule.id, payload, totpCode);
    if (response.success) {
      showToast(copy.alertsPage.ruleCreated);
      await load(true);
    } else {
      setError(responseError(response));
    }
  }

  async function deleteRule(rule: AlertRule) {
    if (
      !(await dialogs.confirm({
        message: copy.alertsPage.deleteRuleConfirm.replace("{name}", String(rule.name)),
        danger: true,
      }))
    ) {
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await apiClient.deleteAlertRule(rule.id, totpCode);
    if (response.success) {
      if (editingRule?.id === rule.id) {
        setShowRuleForm(false);
        setEditingRule(null);
      }
      showToast(copy.alertsPage.ruleDeleted);
      await load(true);
    } else {
      setError(responseError(response));
    }
  }

  async function submitChannel() {
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const payload: JsonObject = {
      name: channelForm.name.trim(),
      request_method: channelForm.request_method,
      request_type: channelForm.request_type,
      verify_tls: channelForm.verify_tls,
      format_metric_units: channelForm.format_metric_units,
    };
    if (
      !editingChannel ||
      channelForm.url !== channelOriginals.url ||
      !isRedactedValue(channelForm.url)
    ) {
      payload["url"] = channelForm.url.trim();
    }
    if (
      !editingChannel ||
      channelForm.headers_json !== channelOriginals.headers_json ||
      !isRedactedValue(channelForm.headers_json)
    ) {
      payload["headers_json"] = channelForm.headers_json.trim() || null;
    }
    if (
      !editingChannel ||
      channelForm.body_template !== channelOriginals.body_template ||
      !isRedactedValue(channelForm.body_template)
    ) {
      payload["body_template"] = channelForm.body_template;
    }
    const response = editingChannel
      ? await updateNotificationChannel(editingChannel.id, payload, totpCode)
      : await createNotificationChannel(payload, totpCode);

    if (response.success) {
      showToast(
        editingChannel
          ? copy.notificationsPage.channelUpdated
          : copy.notificationsPage.channelCreated,
      );
      setShowChannelForm(false);
      setEditingChannel(null);
      await load(true);
    } else {
      setError(responseError(response));
    }
  }

  async function handleTestChannel(channel: NotificationChannel) {
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await testNotificationChannel(channel.id, totpCode);
    if (response.success) {
      showToast(copy.notificationsPage.testSent.replace("{name}", String(channel.name)));
    } else {
      setError(responseError(response));
    }
  }

  async function handleDeleteChannel(channel: NotificationChannel) {
    if (
      !(await dialogs.confirm({
        message: copy.notificationsPage.deleteChannelConfirm.replace("{name}", String(channel.name)),
        danger: true,
      }))
    ) {
      return;
    }
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    const response = await deleteNotificationChannel(channel.id, totpCode);
    if (response.success) {
      if (editingChannel?.id === channel.id) {
        setShowChannelForm(false);
        setEditingChannel(null);
      }
      showToast(copy.notificationsPage.channelDeleted);
      await load(true);
    } else {
      setError(responseError(response));
    }
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
      setError(copy.alertsPage.invalidTotp);
      return null;
    }
    return trimmed;
  }

  return (
    <div>
      <PageShell wide>
        <PageHeader
          eyebrow={copy.alertsPage.eyebrow}
          title={copy.alertsPage.title}
          detail={copy.alertsPage.detail}
          actions={
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                className={buttonClass("secondary")}
                onClick={() => void load(true)}
              >
                {copy.notificationsPage.refresh}
              </button>
              <button
                type="button"
                className={buttonClass(showChannelForm ? "primary" : "secondary")}
                onClick={toggleChannelDrawer}
              >
                {isZh ? "+ 新建通知渠道" : "+ New Channel"}
              </button>
              <button
                type="button"
                className={buttonClass(showRuleForm ? "secondary" : "primary")}
                onClick={toggleRuleDrawer}
              >
                {isZh ? "+ 新建告警规则" : "+ New Alert Rule"}
              </button>
            </div>
          }
        />

        <div className="mb-4 space-y-2.5">
          <InlineError message={error} />
        </div>

        {/* 1. Top 3 KPI Cards */}
        <div className="mb-5 grid gap-4 sm:grid-cols-3">
          <BrutalCard
            className={`flex flex-col justify-between p-4 ${
              kpi.activeIncidents > 0 ? "border-rose-500/40" : ""
            }`}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-[var(--text-muted)]">
                {isZh ? "活跃告警与事件流" : "Active Incidents & Events"}
              </span>
              <StatusBadge tone={kpi.activeIncidents > 0 ? "red" : "green"}>
                {kpi.activeIncidents > 0
                  ? `${kpi.activeIncidents} ${isZh ? "触发中" : "Firing"}`
                  : isZh
                    ? "无活跃异常"
                    : "All Clear"}
              </StatusBadge>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span
                className={`font-mono text-2xl font-semibold tracking-tight tabular-nums ${
                  kpi.activeIncidents > 0
                    ? "text-rose-600 dark:text-rose-400"
                    : "text-[var(--text-main)]"
                }`}
              >
                {kpi.activeIncidents}
                <span className="ml-1.5 text-sm font-normal text-[var(--text-muted)]">
                  / {kpi.totalEvents} {isZh ? "条近期记录" : "recent events"}
                </span>
              </span>
              <button
                type="button"
                onClick={() => setActiveTab("events")}
                className="text-xs font-medium text-[var(--accent-color)] hover:underline"
              >
                {isZh ? "事件流 →" : "Stream →"}
              </button>
            </div>
            <div className="mt-2.5 border-t border-[var(--border-color)] pt-2 text-xs text-[var(--text-muted)]">
              {kpi.activeIncidents > 0
                ? isZh
                  ? "存在尚未恢复的触发事件，请及时排查"
                  : "Unresolved alert events detected in stream"
                : isZh
                  ? "所有监控指标均处于阈值安全范围内"
                  : "All monitored thresholds operating within bounds"}
            </div>
          </BrutalCard>

          <BrutalCard className="flex flex-col justify-between p-4">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-[var(--text-muted)]">
                {isZh ? "告警规则引擎" : "Alert Rules Engine"}
              </span>
              <StatusBadge tone={kpi.enabledRules > 0 ? "blue" : "gray"}>
                {kpi.enabledRules} / {kpi.totalRules} {copy.alertsPage.enabled}
              </StatusBadge>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span className="font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
                {kpi.totalRules}
                <span className="ml-1.5 text-sm font-normal text-[var(--text-muted)]">
                  {isZh ? "条策略" : "rules"} ({formatPercent(kpi.ruleRatio)})
                </span>
              </span>
              <button
                type="button"
                onClick={() => setActiveTab("rules")}
                className="text-xs font-medium text-[var(--accent-color)] hover:underline"
              >
                {isZh ? "管理规则 →" : "Rules →"}
              </button>
            </div>
            <div className="mt-3">
              <ProgressBar value={kpi.totalRules > 0 ? kpi.ruleRatio : 0} tone="emerald" />
            </div>
          </BrutalCard>

          <BrutalCard className="flex flex-col justify-between p-4">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-[var(--text-muted)]">
                {isZh ? "通知渠道与投递组" : "Notification Channels"}
              </span>
              <StatusBadge tone={kpi.totalChannels > 0 ? "pink" : "gray"}>
                {kpi.totalGroups} {isZh ? "个通知组" : "groups"}
              </StatusBadge>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-2">
              <span className="font-mono text-2xl font-semibold tracking-tight text-[var(--text-main)] tabular-nums">
                {kpi.totalChannels}
                <span className="ml-1.5 text-sm font-normal text-[var(--text-muted)]">
                  {isZh ? "个 Webhook 渠道" : "channels"}
                </span>
              </span>
              <button
                type="button"
                onClick={() => setActiveTab("channels")}
                className="text-xs font-medium text-[var(--accent-color)] hover:underline"
              >
                {isZh ? "渠道测试 →" : "Channels →"}
              </button>
            </div>
            <div className="mt-2.5 border-t border-[var(--border-color)] pt-2 text-xs text-[var(--text-muted)]">
              {isZh
                ? `${kpi.tlsVerifiedChannels} 个启用 TLS 校验 · 支持一键连通性测试`
                : `${kpi.tlsVerifiedChannels} TLS verified · One-click delivery test ready`}
            </div>
          </BrutalCard>
        </div>

        {/* 2. Collapsible Creation / Edit Drawers */}
        {showRuleForm ? (
          <BrutalCard className="mb-6 border-[var(--accent-color)]/40 bg-[var(--bg-card)]">
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3.5">
              <div className="flex items-center gap-2.5">
                <span className="h-2.5 w-2.5 rounded-full bg-[var(--accent-color)]" />
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {editingRule
                    ? `${isZh ? "编辑告警规则" : "Edit Alert Rule"} — ${editingRule.name}`
                    : copy.alertsPage.modalTitle}
                </h2>
                <StatusBadge tone={editingRule ? "yellow" : "blue"}>
                  {editingRule ? (isZh ? "编辑策略" : "Editing") : isZh ? "新建策略" : "New Rule"}
                </StatusBadge>
              </div>
              <button
                type="button"
                onClick={() => {
                  setShowRuleForm(false);
                  setEditingRule(null);
                }}
                className={buttonClass("secondary")}
              >
                {copy.common.close}
              </button>
            </div>

            <form
              onSubmit={(event) => {
                event.preventDefault();
                void runForm(() => submitRule());
              }}
              className="space-y-4"
            >
              <div className="grid gap-4 sm:grid-cols-3">
                <Field label={copy.alertsPage.fieldName}>
                  <input
                    className={inputClass}
                    value={form.name}
                    onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
                    placeholder={isZh ? "如：CPU 持续高负载告警" : "e.g. High CPU Load Alert"}
                    required
                  />
                </Field>
                <Field label={copy.alertsPage.fieldTrigger}>
                  <select
                    className={selectClass}
                    value={form.trigger}
                    onChange={(e) => setForm((f) => ({ ...f, trigger: e.target.value }))}
                  >
                    <option value="once">once ({isZh ? "单次触发" : "Fire once per incident"})</option>
                    <option value="always">always ({isZh ? "持续触发" : "Fire on every match"})</option>
                  </select>
                </Field>
                <Field label={copy.alertsPage.fieldConditionType}>
                  <select
                    className={selectClass}
                    value={form.condition_type}
                    onChange={(e) => setForm((f) => ({ ...f, condition_type: e.target.value }))}
                  >
                    <option value="server_resource">server_resource</option>
                    <option value="server_offline">server_offline</option>
                    <option value="server_expiry">server_expiry</option>
                    <option value="server_traffic_quota">server_traffic_quota</option>
                    <option value="service_down">service_down</option>
                    <option value="service_latency">service_latency</option>
                    <option value="certificate_expiry">certificate_expiry</option>
                  </select>
                </Field>
              </div>

              <div className="grid gap-4 sm:grid-cols-2">
                {form.condition_type.startsWith("server") ? (
                  <Field label={copy.alertsPage.fieldAgentId}>
                    <input
                      className={`${inputClass} font-mono`}
                      value={form.agent_id}
                      onChange={(e) => setForm((f) => ({ ...f, agent_id: e.target.value }))}
                      placeholder={isZh ? "输入目标节点 Agent ID 或 *" : "Target Agent ID or *"}
                      required
                    />
                  </Field>
                ) : (
                  <Field label={copy.alertsPage.fieldServiceId}>
                    <input
                      className={`${inputClass} font-mono`}
                      value={form.service_id}
                      onChange={(e) => setForm((f) => ({ ...f, service_id: e.target.value }))}
                      placeholder={isZh ? "输入目标服务监控 ID" : "Target Service ID"}
                      required
                    />
                  </Field>
                )}

                <Field label={copy.alertsPage.fieldNotificationGroupId}>
                  {groups.length > 0 ? (
                    <select
                      className={selectClass}
                      value={form.notification_group_id}
                      onChange={(e) => setForm((f) => ({ ...f, notification_group_id: e.target.value }))}
                    >
                      <option value="">{isZh ? "不绑定 / 默认通知组" : "None / Default Group"}</option>
                      {groups.map((group) => (
                        <option key={group.id} value={group.id}>
                          {group.name} ({group.id})
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      className={inputClass}
                      value={form.notification_group_id}
                      onChange={(e) => setForm((f) => ({ ...f, notification_group_id: e.target.value }))}
                    />
                  )}
                </Field>
              </div>

              {form.condition_type === "server_resource" ? (
                <div className="grid gap-4 sm:grid-cols-4">
                  <Field label={copy.alertsPage.fieldResource}>
                    <select
                      className={selectClass}
                      value={form.resource}
                      onChange={(e) => setForm((f) => ({ ...f, resource: e.target.value }))}
                    >
                      <option value="cpu">cpu</option>
                      <option value="memory">memory</option>
                      <option value="disk">disk</option>
                      <option value="swap">swap</option>
                      <option value="network">network_delta</option>
                      <option value="network_in">network_in</option>
                      <option value="network_out">network_out</option>
                      <option value="network_total">network_total</option>
                      <option value="traffic_in_total">traffic_in_total</option>
                      <option value="traffic_out_total">traffic_out_total</option>
                      <option value="load">load1</option>
                      <option value="load5">load5</option>
                      <option value="load15">load15</option>
                      <option value="tcp">tcp</option>
                      <option value="udp">udp</option>
                      <option value="process">process</option>
                      <option value="temperature">temperature</option>
                      <option value="gpu">gpu</option>
                    </select>
                  </Field>
                  <Field label={copy.alertsPage.fieldOperator}>
                    <select
                      className={selectClass}
                      value={form.operator}
                      onChange={(e) => setForm((f) => ({ ...f, operator: e.target.value }))}
                    >
                      <option value="gt">gt (&gt;)</option>
                      <option value="gte">gte (&gt;=)</option>
                      <option value="lt">lt (&lt;)</option>
                      <option value="lte">lte (&lt;=)</option>
                    </select>
                  </Field>
                  <Field label={copy.alertsPage.fieldThreshold}>
                    <input
                      className={inputClass}
                      value={form.threshold}
                      onChange={(e) => setForm((f) => ({ ...f, threshold: e.target.value }))}
                    />
                  </Field>
                  <Field label={copy.alertsPage.fieldDurationSeconds}>
                    <input
                      className={inputClass}
                      value={form.duration_seconds}
                      onChange={(e) => setForm((f) => ({ ...f, duration_seconds: e.target.value }))}
                    />
                  </Field>
                </div>
              ) : null}

              {form.condition_type === "server_offline" ? (
                <Field label={copy.alertsPage.fieldOfflineSeconds}>
                  <input
                    className={inputClass}
                    value={form.offline_seconds}
                    onChange={(e) => setForm((f) => ({ ...f, offline_seconds: e.target.value }))}
                  />
                </Field>
              ) : null}
              {form.condition_type === "server_expiry" ? (
                <Field label={copy.alertsPage.fieldDaysBefore}>
                  <input
                    className={inputClass}
                    value={form.server_expiry_days_before}
                    onChange={(e) => setForm((f) => ({ ...f, server_expiry_days_before: e.target.value }))}
                  />
                </Field>
              ) : null}
              {form.condition_type === "server_traffic_quota" ? (
                <div className="grid gap-4 sm:grid-cols-2">
                  <Field label={copy.alertsPage.fieldUsagePercent}>
                    <input
                      className={inputClass}
                      value={form.traffic_quota_percent}
                      onChange={(e) => setForm((f) => ({ ...f, traffic_quota_percent: e.target.value }))}
                    />
                  </Field>
                  <Field label={copy.alertsPage.fieldDirection}>
                    <select
                      className={selectClass}
                      value={form.traffic_quota_direction}
                      onChange={(e) => setForm((f) => ({ ...f, traffic_quota_direction: e.target.value }))}
                    >
                      <option value="total">total</option>
                      <option value="in">in</option>
                      <option value="out">out</option>
                    </select>
                  </Field>
                </div>
              ) : null}
              {form.condition_type === "service_down" ? (
                <Field label={copy.alertsPage.fieldConsecutiveFailures}>
                  <input
                    className={inputClass}
                    value={form.consecutive_failures}
                    onChange={(e) => setForm((f) => ({ ...f, consecutive_failures: e.target.value }))}
                  />
                </Field>
              ) : null}
              {form.condition_type === "service_latency" ? (
                <Field label={copy.alertsPage.fieldMaxLatencyMs}>
                  <input
                    className={inputClass}
                    value={form.max_latency_ms}
                    onChange={(e) => setForm((f) => ({ ...f, max_latency_ms: e.target.value }))}
                  />
                </Field>
              ) : null}
              {form.condition_type === "certificate_expiry" ? (
                <Field label={copy.alertsPage.fieldDaysBefore}>
                  <input
                    className={inputClass}
                    value={form.cert_days_before}
                    onChange={(e) => setForm((f) => ({ ...f, cert_days_before: e.target.value }))}
                  />
                </Field>
              ) : null}

              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.alertsPage.fieldFailureTaskIds}>
                  <input
                    className={inputClass}
                    value={form.failure_task_ids}
                    onChange={(e) => setForm((f) => ({ ...f, failure_task_ids: e.target.value }))}
                  />
                </Field>
                <Field label={copy.alertsPage.fieldRecoveryTaskIds}>
                  <input
                    className={inputClass}
                    value={form.recovery_task_ids}
                    onChange={(e) => setForm((f) => ({ ...f, recovery_task_ids: e.target.value }))}
                  />
                </Field>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-color)] pt-3.5">
                <label className="flex items-center gap-2.5 text-sm font-medium text-[var(--text-main)]">
                  <input
                    type="checkbox"
                    checked={form.enabled}
                    onChange={(e) => setForm((f) => ({ ...f, enabled: e.target.checked }))}
                  />
                  {copy.alertsPage.enabled}
                </label>

                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    onClick={() => {
                      setShowRuleForm(false);
                      setEditingRule(null);
                    }}
                    className={buttonClass("ghost")}
                  >
                    {copy.common.cancel}
                  </button>
                  <button type="submit" disabled={formBusy} className={buttonClass("primary")}>
                    {copy.alertsPage.saveRule}
                  </button>
                </div>
              </div>
            </form>
          </BrutalCard>
        ) : null}

        {showChannelForm ? (
          <BrutalCard className="mb-6 border-[var(--accent-color)]/40 bg-[var(--bg-card)]">
            <div className="mb-4 flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3.5">
              <div className="flex items-center gap-2.5">
                <span className="h-2.5 w-2.5 rounded-full bg-fuchsia-500" />
                <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                  {editingChannel
                    ? `${copy.notificationsPage.editChannel} — ${editingChannel.name}`
                    : copy.notificationsPage.createChannel}
                </h2>
                <StatusBadge tone="pink">Webhook</StatusBadge>
              </div>
              <button
                type="button"
                onClick={() => {
                  setShowChannelForm(false);
                  setEditingChannel(null);
                }}
                className={buttonClass("secondary")}
              >
                {copy.common.close}
              </button>
            </div>

            <form
              onSubmit={(event) => {
                event.preventDefault();
                void runForm(() => submitChannel());
              }}
              className="space-y-4"
            >
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                <Field label={copy.notificationsPage.fieldName}>
                  <input
                    className={inputClass}
                    value={channelForm.name}
                    onChange={(event) =>
                      setChannelForm((prev) => ({ ...prev, name: event.target.value }))
                    }
                    required
                  />
                </Field>
                <Field label={copy.notificationsPage.fieldProvider}>
                  <select
                    className={selectClass}
                    onChange={(event) => applyProvider(event.target.value)}
                    defaultValue=""
                  >
                    <option value="">{copy.notificationsPage.keepCurrentTemplate}</option>
                    {providers.map((provider) => (
                      <option key={provider.id} value={provider.id}>
                        {provider.name}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label={copy.notificationsPage.fieldMethod}>
                  <select
                    className={selectClass}
                    value={channelForm.request_method}
                    onChange={(event) =>
                      setChannelForm((prev) => ({ ...prev, request_method: event.target.value }))
                    }
                  >
                    <option value="POST">POST</option>
                    <option value="GET">GET</option>
                    <option value="PUT">PUT</option>
                    <option value="PATCH">PATCH</option>
                  </select>
                </Field>
                <Field label={copy.notificationsPage.fieldType}>
                  <select
                    className={selectClass}
                    value={channelForm.request_type}
                    onChange={(event) =>
                      setChannelForm((prev) => ({ ...prev, request_type: event.target.value }))
                    }
                  >
                    <option value="json">json</option>
                    <option value="form">form</option>
                    <option value="custom">custom</option>
                  </select>
                </Field>
              </div>

              <Field label={copy.notificationsPage.fieldUrl}>
                <input
                  className={`${inputClass} font-mono`}
                  value={channelForm.url}
                  onChange={(event) =>
                    setChannelForm((prev) => ({ ...prev, url: event.target.value }))
                  }
                  placeholder="https://hooks.example.com/services/..."
                  required
                />
              </Field>

              <div className="grid gap-4 sm:grid-cols-2">
                <Field label={copy.notificationsPage.fieldHeadersJson}>
                  <textarea
                    className={`${textareaClass} min-h-24`}
                    value={channelForm.headers_json}
                    onChange={(event) =>
                      setChannelForm((prev) => ({ ...prev, headers_json: event.target.value }))
                    }
                    spellCheck={false}
                  />
                </Field>
                <Field label={copy.notificationsPage.fieldBodyTemplate}>
                  <textarea
                    className={`${textareaClass} min-h-24`}
                    value={channelForm.body_template}
                    onChange={(event) =>
                      setChannelForm((prev) => ({ ...prev, body_template: event.target.value }))
                    }
                    spellCheck={false}
                  />
                </Field>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-color)] pt-3.5">
                <div className="flex flex-wrap gap-4">
                  <label className="flex items-center gap-2 text-sm font-medium text-[var(--text-main)]">
                    <input
                      type="checkbox"
                      checked={channelForm.verify_tls}
                      onChange={(event) =>
                        setChannelForm((prev) => ({ ...prev, verify_tls: event.target.checked }))
                      }
                    />
                    {copy.notificationsPage.verifyTlsCert}
                  </label>
                  <label className="flex items-center gap-2 text-sm font-medium text-[var(--text-main)]">
                    <input
                      type="checkbox"
                      checked={channelForm.format_metric_units}
                      onChange={(event) =>
                        setChannelForm((prev) => ({
                          ...prev,
                          format_metric_units: event.target.checked,
                        }))
                      }
                    />
                    {copy.notificationsPage.formatMetricUnits}
                  </label>
                </div>

                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    onClick={() => {
                      setShowChannelForm(false);
                      setEditingChannel(null);
                    }}
                    className={buttonClass("ghost")}
                  >
                    {copy.common.cancel}
                  </button>
                  <button type="submit" disabled={formBusy} className={buttonClass("primary")}>
                    {copy.notificationsPage.saveChannel}
                  </button>
                </div>
              </div>
            </form>
          </BrutalCard>
        ) : null}

        {/* 3. Segmented Workspace Mode Tabs Bar */}
        <div className="mb-5 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-2.5 shadow-[var(--shadow-xs)]">
          <div className="inline-flex flex-wrap gap-1 rounded-lg bg-[var(--bg-subtle)] p-1">
            {(
              [
                {
                  id: "overview",
                  label: isZh ? "全景概览" : "Overview",
                  badge: null,
                },
                {
                  id: "events",
                  label: isZh ? "告警事件流" : "Event Stream",
                  badge: events.length,
                },
                {
                  id: "rules",
                  label: isZh ? "告警规则引擎" : "Alert Rules",
                  badge: rules.length,
                },
                {
                  id: "channels",
                  label: isZh ? "通知渠道" : "Channels",
                  badge: channels.length,
                },
              ] as const
            ).map((tab) => {
              const active = activeTab === tab.id;
              return (
                <button
                  key={tab.id}
                  type="button"
                  onClick={() => setActiveTab(tab.id)}
                  className={`inline-flex items-center gap-2 rounded-md px-3 py-1.5 text-xs font-medium transition ${
                    active
                      ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                      : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                  }`}
                >
                  <span>{tab.label}</span>
                  {tab.badge !== null ? (
                    <span className="rounded-full bg-[var(--bg-muted)] px-1.5 py-0.2 font-mono text-[10px] text-[var(--text-muted)]">
                      {tab.badge}
                    </span>
                  ) : null}
                </button>
              );
            })}
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={toggleChannelDrawer}
              className={buttonClass("secondary")}
            >
              {isZh ? "+ 新建通知渠道" : "+ New Channel"}
            </button>
            <button
              type="button"
              onClick={toggleRuleDrawer}
              className={buttonClass("primary")}
            >
              {isZh ? "+ 新建告警规则" : "+ New Alert Rule"}
            </button>
          </div>
        </div>

        {/* 4. Workspace Content Views */}
        {activeTab === "overview" ? (
          <div className="grid gap-6 xl:grid-cols-[minmax(0,1.25fr)_minmax(0,1fr)]">
            {/* Left: Alert Rules Engine + Notification Channels */}
            <div className="space-y-6">
              <BrutalCard>
                <div className="mb-4 flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                      {copy.alertsPage.rulesHeading}
                    </h2>
                    <StatusBadge tone="blue">{rules.length}</StatusBadge>
                  </div>
                  <button
                    type="button"
                    onClick={() => setActiveTab("rules")}
                    className="text-xs font-medium text-[var(--accent-color)] hover:underline"
                  >
                    {isZh ? "查看全部规则 →" : "All Rules →"}
                  </button>
                </div>

                {rules.length === 0 ? (
                  <EmptyState
                    title={copy.alertsPage.noRules}
                    action={
                      <button type="button" onClick={toggleRuleDrawer} className={buttonClass("primary")}>
                        {isZh ? "+ 新建告警规则" : "+ New Alert Rule"}
                      </button>
                    }
                  />
                ) : (
                  <div className="grid gap-2.5">
                    {rules.slice(0, 6).map((rule) => (
                      <div
                        key={rule.id}
                        className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)]/70 p-3.5 transition-colors hover:border-[var(--border-strong)]"
                      >
                        <div className="min-w-0 space-y-1.5">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="font-semibold text-sm text-[var(--text-main)]">
                              {rule.name}
                            </span>
                            <StatusBadge tone={rule.enabled === false ? "gray" : "green"}>
                              {rule.enabled === false ? copy.alertsPage.disabled : copy.alertsPage.enabled}
                            </StatusBadge>
                            {rule.trigger ? (
                              <span className="rounded-md border border-[var(--border-color)] bg-[var(--bg-card)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                                {rule.trigger}
                              </span>
                            ) : null}
                          </div>
                          <ConditionPills conditions={rule.conditions} copy={copy} />
                        </div>

                        <div className="flex flex-wrap items-center gap-1.5">
                          <button
                            type="button"
                            disabled={rowBusy}
                            onClick={() => void runRow(() => toggleRuleEnabled(rule))}
                            className={buttonClass("secondary")}
                          >
                            {rule.enabled === false ? copy.alertsPage.enabled : copy.alertsPage.disabled}
                          </button>
                          <button
                            type="button"
                            onClick={() => openEditRule(rule)}
                            className={buttonClass("secondary")}
                          >
                            {copy.notificationsPage.edit}
                          </button>
                          <button
                            type="button"
                            disabled={rowBusy}
                            onClick={() => void runRow(() => deleteRule(rule))}
                            className={buttonClass("danger")}
                          >
                            {copy.alertsPage.delete}
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </BrutalCard>

              <BrutalCard>
                <div className="mb-4 flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                      {copy.notificationsPage.channels}
                    </h2>
                    <StatusBadge tone="pink">{channels.length}</StatusBadge>
                  </div>
                  <button
                    type="button"
                    onClick={() => setActiveTab("channels")}
                    className="text-xs font-medium text-[var(--accent-color)] hover:underline"
                  >
                    {isZh ? "管理通知渠道 →" : "Manage Channels →"}
                  </button>
                </div>

                {channels.length === 0 ? (
                  <EmptyState
                    title={copy.notificationsPage.noChannels}
                    action={
                      <button
                        type="button"
                        onClick={toggleChannelDrawer}
                        className={buttonClass("secondary")}
                      >
                        {isZh ? "+ 新建通知渠道" : "+ New Channel"}
                      </button>
                    }
                  />
                ) : (
                  <div className="grid gap-2.5 sm:grid-cols-2">
                    {channels.slice(0, 6).map((channel) => (
                      <div
                        key={channel.id}
                        className="flex flex-col justify-between gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)]/70 p-3.5"
                      >
                        <div className="min-w-0 space-y-1">
                          <div className="flex items-center justify-between gap-2">
                            <span className="truncate text-sm font-semibold text-[var(--text-main)]">
                              {channel.name}
                            </span>
                            <div className="flex shrink-0 gap-1">
                              <StatusBadge tone="gray">{channel.request_method || "POST"}</StatusBadge>
                              <StatusBadge tone="blue">{channel.request_type || "json"}</StatusBadge>
                            </div>
                          </div>
                          <div className="truncate font-mono text-xs text-[var(--text-muted)]">
                            {channel.url}
                          </div>
                        </div>

                        <div className="flex items-center justify-between gap-2 border-t border-[var(--border-color)] pt-2.5">
                          <StatusBadge tone={channel.verify_tls === false ? "yellow" : "green"}>
                            TLS: {channel.verify_tls === false ? copy.notificationsPage.tlsSkip : copy.notificationsPage.tlsVerify}
                          </StatusBadge>
                          <div className="flex gap-1.5">
                            <button
                              type="button"
                              disabled={channelBusy}
                              onClick={() => void runChannel(() => handleTestChannel(channel))}
                              className={buttonClass("secondary")}
                            >
                              {isZh ? "测试发送" : copy.notificationsPage.test}
                            </button>
                            <button
                              type="button"
                              onClick={() => openEditChannel(channel)}
                              className={buttonClass("ghost")}
                            >
                              {copy.notificationsPage.edit}
                            </button>
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </BrutalCard>
            </div>

            {/* Right: Live Incident & Event Stream */}
            <BrutalCard className="flex flex-col">
              <div className="mb-4 flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <h2 className="text-base font-semibold tracking-tight text-[var(--text-main)]">
                    {copy.alertsPage.eventsHeading}
                  </h2>
                  <StatusBadge tone={kpi.activeIncidents > 0 ? "red" : "gray"}>
                    {events.length}
                  </StatusBadge>
                </div>
                <button
                  type="button"
                  onClick={() => setActiveTab("events")}
                  className="text-xs font-medium text-[var(--accent-color)] hover:underline"
                >
                  {isZh ? "筛选与搜索 →" : "Filter Stream →"}
                </button>
              </div>

              {events.length === 0 ? (
                <EmptyState title={copy.alertsPage.noEvents} />
              ) : (
                <div className="grid gap-2.5">
                  {events.slice(0, 8).map((event, index) => (
                    <EventCard key={event.id || index} event={event} copy={copy} />
                  ))}
                </div>
              )}
            </BrutalCard>
          </div>
        ) : null}

        {activeTab === "events" ? (
          <div className="space-y-4">
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-3 shadow-[var(--shadow-xs)]">
              <div className="min-w-[14rem] flex-1">
                <input
                  value={eventQuery}
                  onChange={(e) => setEventQuery(e.target.value)}
                  className="block w-full rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] px-3 py-1.5 text-sm text-[var(--text-main)] outline-none transition placeholder:text-[var(--text-faint)] focus:border-[var(--accent-color)] focus:bg-[var(--bg-card)]"
                  placeholder={isZh ? "搜索事件名称、内容或状态..." : "Search events by rule, payload, or status..."}
                />
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <div className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5">
                  {(
                    [
                      { id: "all", label: isZh ? "全部状态" : "All" },
                      { id: "firing", label: isZh ? "触发中" : "Firing" },
                      { id: "resolved", label: isZh ? "已恢复" : "Resolved" },
                    ] as const
                  ).map((pill) => (
                    <button
                      key={pill.id}
                      type="button"
                      onClick={() => setEventStatusFilter(pill.id)}
                      className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                        eventStatusFilter === pill.id
                          ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                          : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {pill.label}
                    </button>
                  ))}
                </div>

                <div className="inline-flex rounded-lg border border-[var(--border-color)] bg-[var(--bg-subtle)] p-0.5">
                  {(
                    [
                      { id: "all", label: isZh ? "全部级别" : "All Severity" },
                      { id: "critical", label: "Critical" },
                      { id: "warning", label: "Warning" },
                      { id: "info", label: "Info" },
                    ] as const
                  ).map((pill) => (
                    <button
                      key={pill.id}
                      type="button"
                      onClick={() => setEventSeverityFilter(pill.id)}
                      className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                        eventSeverityFilter === pill.id
                          ? "bg-[var(--bg-card)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                          : "text-[var(--text-muted)] hover:text-[var(--text-main)]"
                      }`}
                    >
                      {pill.label}
                    </button>
                  ))}
                </div>
              </div>
            </div>

            {filteredEvents.length === 0 ? (
              <EmptyState title={copy.alertsPage.noEvents} />
            ) : (
              <div className="grid gap-3">
                {filteredEvents.map((event, index) => (
                  <EventCard key={event.id || index} event={event} copy={copy} />
                ))}
              </div>
            )}
          </div>
        ) : null}

        {activeTab === "rules" ? (
          <div>
            {rules.length === 0 ? (
              <EmptyState
                title={copy.alertsPage.noRules}
                action={
                  <button type="button" onClick={toggleRuleDrawer} className={buttonClass("primary")}>
                    {isZh ? "+ 新建告警规则" : "+ New Alert Rule"}
                  </button>
                }
              />
            ) : (
              <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]">
                <table className="w-full min-w-[56rem]">
                  <thead>
                    <tr>
                      <th className={thClass}>{copy.alertsPage.colName}</th>
                      <th className={thClass}>{copy.alertsPage.colCondition}</th>
                      <th className={thClass}>{copy.alertsPage.fieldTrigger}</th>
                      <th className={thClass}>{copy.alertsPage.colStatus}</th>
                      <th className={thClass}>{copy.alertsPage.colActions}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-[var(--border-color)]">
                    {rules.map((rule) => (
                      <tr key={rule.id} className="transition-colors hover:bg-[var(--bg-subtle)]">
                        <td className={tdClass}>
                          <div className="font-semibold text-[var(--text-main)]">{rule.name}</div>
                          {rule.created_at ? (
                            <div className="mt-0.5 font-mono text-[11px] text-[var(--text-muted)]">
                              {formatDate(rule.created_at)}
                            </div>
                          ) : null}
                        </td>
                        <td className={tdClass}>
                          <ConditionPills conditions={rule.conditions} copy={copy} />
                        </td>
                        <td className={tdClass}>
                          <span className="rounded-md border border-[var(--border-color)] bg-[var(--bg-subtle)] px-2 py-0.5 font-mono text-xs text-[var(--text-main)]">
                            {rule.trigger || "once"}
                          </span>
                        </td>
                        <td className={tdClass}>
                          <StatusBadge tone={rule.enabled === false ? "gray" : "green"}>
                            {rule.enabled === false ? copy.alertsPage.disabled : copy.alertsPage.enabled}
                          </StatusBadge>
                        </td>
                        <td className={tdClass}>
                          <div className="flex flex-wrap gap-2">
                            <button
                              type="button"
                              disabled={rowBusy}
                              onClick={() => void runRow(() => toggleRuleEnabled(rule))}
                              className={buttonClass("secondary")}
                            >
                              {rule.enabled === false ? copy.alertsPage.enabled : copy.alertsPage.disabled}
                            </button>
                            <button
                              type="button"
                              onClick={() => openEditRule(rule)}
                              className={buttonClass("secondary")}
                            >
                              {copy.notificationsPage.edit}
                            </button>
                            <button
                              type="button"
                              className={buttonClass("danger")}
                              disabled={rowBusy}
                              onClick={() => void runRow(() => deleteRule(rule))}
                            >
                              {copy.alertsPage.delete}
                            </button>
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        ) : null}

        {activeTab === "channels" ? (
          <div>
            {channels.length === 0 ? (
              <EmptyState
                title={copy.notificationsPage.noChannels}
                action={
                  <button
                    type="button"
                    onClick={toggleChannelDrawer}
                    className={buttonClass("primary")}
                  >
                    {isZh ? "+ 新建通知渠道" : "+ New Channel"}
                  </button>
                }
              />
            ) : (
              <div className="overflow-x-auto rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] shadow-[var(--shadow-xs)]">
                <table className="w-full min-w-[60rem]">
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
                  <tbody className="divide-y divide-[var(--border-color)]">
                    {channels.map((channel) => (
                      <tr key={channel.id} className="transition-colors hover:bg-[var(--bg-subtle)]">
                        <td className={`${tdClass} font-semibold`}>{channel.name}</td>
                        <td className={tdClass}>
                          <div className="flex flex-wrap gap-1.5">
                            <StatusBadge tone="gray">{channel.request_method || "POST"}</StatusBadge>
                            <StatusBadge tone="blue">{channel.request_type || "json"}</StatusBadge>
                          </div>
                        </td>
                        <td className={`${tdClass} max-w-[20rem] break-all font-mono text-xs text-[var(--text-muted)]`}>
                          {channel.url}
                        </td>
                        <td className={tdClass}>
                          <StatusBadge tone={channel.verify_tls === false ? "yellow" : "green"}>
                            {channel.verify_tls === false
                              ? copy.notificationsPage.tlsSkip
                              : copy.notificationsPage.tlsVerify}
                          </StatusBadge>
                        </td>
                        <td className={`${tdClass} font-mono text-xs text-[var(--text-muted)]`}>
                          {formatDate(channel.updated_at || channel.created_at)}
                        </td>
                        <td className={tdClass}>
                          <div className="flex flex-wrap gap-2">
                            <button
                              type="button"
                              disabled={channelBusy}
                              onClick={() => void runChannel(() => handleTestChannel(channel))}
                              className={buttonClass("secondary")}
                            >
                              {isZh ? "测试发送" : copy.notificationsPage.test}
                            </button>
                            <button
                              type="button"
                              onClick={() => openEditChannel(channel)}
                              className={buttonClass("secondary")}
                            >
                              {copy.notificationsPage.edit}
                            </button>
                            <button
                              type="button"
                              disabled={channelBusy}
                              onClick={() => void runChannel(() => handleDeleteChannel(channel))}
                              className={buttonClass("danger")}
                            >
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
          </div>
        ) : null}
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function ConditionPills({
  conditions,
  copy,
}: {
  conditions?: JsonObject[] | undefined;
  copy: Translations;
}) {
  if (!conditions || conditions.length === 0) {
    return <span className="font-mono text-xs text-[var(--text-muted)]">-</span>;
  }

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {conditions.map((condition, idx) => {
        const pill = buildConditionPill(copy, condition);
        return (
          <span
            key={idx}
            className="inline-flex items-center gap-1.5 rounded-md border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1 font-mono text-xs text-[var(--text-main)] shadow-[var(--shadow-xs)]"
          >
            {pill.target ? (
              <span className="rounded-xs bg-[var(--bg-muted)] px-1.5 py-0.2 text-[11px] text-[var(--text-muted)]">
                {pill.target}
              </span>
            ) : null}
            <span className="font-semibold text-[var(--accent-color)]">{pill.expression}</span>
          </span>
        );
      })}
    </div>
  );
}

function EventCard({
  event,
  copy,
}: {
  event: AlertEvent;
  copy: Translations;
}) {
  const status = extractEventStatus(event);
  const severity = extractEventSeverity(event);
  const title = extractEventTitle(copy, event);
  const timestamp = event.fired_at || event.created_at;

  return (
    <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)]/70 p-4 transition-colors hover:border-[var(--border-strong)]">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-[var(--text-main)]">{title}</span>
          <StatusBadge
            tone={
              severity === "critical"
                ? "red"
                : severity === "warning"
                  ? "yellow"
                  : "blue"
            }
          >
            {severity}
          </StatusBadge>
        </div>
        <StatusBadge tone={status === "resolved" ? "green" : "red"}>
          {status}
        </StatusBadge>
      </div>
      <p className="mt-1.5 break-all font-mono text-xs text-[var(--text-muted)]">
        {formatPayload(copy, event.payload ?? event.message)}
      </p>
      <p className="mt-2 font-mono text-[11px] text-[var(--text-muted)]">
        {formatDate(timestamp)}
      </p>
    </div>
  );
}

function extractEventStatus(event: AlertEvent): string {
  if (event.status) return event.status;
  const rec = asRecord(event.payload);
  const payloadStatus = asString(rec["status"]);
  if (payloadStatus) return payloadStatus;
  if (event.kind?.toLowerCase().includes("recover") || event.kind?.toLowerCase().includes("resolve")) {
    return "resolved";
  }
  return "firing";
}

function extractEventSeverity(event: AlertEvent): "critical" | "warning" | "info" {
  const raw = (event.severity || asString(asRecord(event.payload)["severity"])).toLowerCase();
  if (raw === "critical" || raw === "high" || raw === "error" || raw === "danger") return "critical";
  if (raw === "info" || raw === "low") return "info";
  return "warning";
}

function extractEventTitle(copy: Translations, event: AlertEvent): string {
  if (event.rule_name) return event.rule_name;
  const rec = asRecord(event.payload);
  const ruleName = asString(rec["rule_name"]) || asString(rec["title"]);
  if (ruleName) return ruleName;
  return event.kind || copy.alertsPage.title;
}

function operatorSymbol(op: string): string {
  if (op === "gt") return ">";
  if (op === "gte") return ">=";
  if (op === "lt") return "<";
  if (op === "lte") return "<=";
  return op;
}

function buildConditionPill(
  copy: Translations,
  condition: JsonObject,
): { target: string; expression: string } {
  const t = copy.alertsPage;
  const type = String(condition["type"] || "condition");
  if (type === "server_resource") {
    const resource = String(condition["resource"] ?? "cpu");
    const op = operatorSymbol(String(condition["operator"] ?? "gt"));
    const threshold = String(condition["threshold"] ?? "");
    const duration = condition["duration_seconds"] ? ` • ${String(condition["duration_seconds"])}s` : "";
    return {
      target: String(condition["agent_id"] ?? ""),
      expression: `${resource}_percent ${op} ${threshold}${duration}`,
    };
  }
  if (type === "server_offline") {
    return {
      target: String(condition["agent_id"] ?? ""),
      expression: `${t.conditionOffline} • ${String(condition["offline_seconds"] ?? "")}s`,
    };
  }
  if (type === "server_expiry") {
    return {
      target: String(condition["agent_id"] ?? ""),
      expression: `${String(condition["days_before"] ?? "")} ${t.conditionExpiryDays}`,
    };
  }
  if (type === "server_traffic_quota") {
    return {
      target: String(condition["agent_id"] ?? ""),
      expression: `${t.conditionTraffic} (${String(condition["direction"] || "total")}) >= ${String(condition["percent"] ?? "")}%`,
    };
  }
  if (type === "service_down") {
    return {
      target: String(condition["service_id"] ?? ""),
      expression: `${t.conditionServiceDown} ×${String(condition["consecutive_failures"] ?? "")}`,
    };
  }
  if (type === "service_latency") {
    return {
      target: String(condition["service_id"] ?? ""),
      expression: `${t.conditionLatency} > ${String(condition["max_latency_ms"] ?? "")}ms`,
    };
  }
  if (type === "certificate_expiry") {
    return {
      target: String(condition["service_id"] ?? ""),
      expression: `${t.conditionCert} <= ${String(condition["days_before"] ?? "")} ${t.conditionExpiryDays}`,
    };
  }
  return {
    target: "",
    expression: formatConditions(copy, [condition]),
  };
}

function ruleToFormState(rule: AlertRule): RuleFormState {
  const first = rule.conditions?.[0] ?? {};
  const condType = asString(first["type"], "server_resource");
  return {
    name: rule.name || "",
    trigger: rule.trigger || "once",
    condition_type: condType,
    agent_id: asString(first["agent_id"]),
    service_id: asString(first["service_id"]),
    resource: asString(first["resource"], "cpu"),
    operator: asString(first["operator"], "gt"),
    threshold: String(first["threshold"] ?? "90"),
    duration_seconds: String(first["duration_seconds"] ?? "60"),
    offline_seconds: String(first["offline_seconds"] ?? "120"),
    consecutive_failures: String(first["consecutive_failures"] ?? "3"),
    max_latency_ms: String(first["max_latency_ms"] ?? "1000"),
    cert_days_before: String(first["days_before"] ?? "7"),
    server_expiry_days_before: String(first["days_before"] ?? "7"),
    traffic_quota_percent: String(first["percent"] ?? "80"),
    traffic_quota_direction: asString(first["direction"], "total"),
    notification_group_id: rule.notification_group_id ?? "",
    failure_task_ids: (rule.failure_task_ids ?? []).join(", "),
    recovery_task_ids: (rule.recovery_task_ids ?? []).join(", "),
    enabled: rule.enabled !== false,
  };
}

function buildCondition(form: {
  condition_type: string;
  agent_id: string;
  service_id: string;
  resource: string;
  operator: string;
  threshold: string;
  duration_seconds: string;
  offline_seconds: string;
  consecutive_failures: string;
  max_latency_ms: string;
  cert_days_before: string;
  server_expiry_days_before: string;
  traffic_quota_percent: string;
  traffic_quota_direction: string;
}): JsonObject {
  if (form.condition_type === "server_offline") {
    return {
      type: "server_offline",
      agent_id: form.agent_id.trim(),
      offline_seconds: Number(form.offline_seconds),
    };
  }
  if (form.condition_type === "service_down") {
    return {
      type: "service_down",
      service_id: form.service_id.trim(),
      consecutive_failures: Number(form.consecutive_failures),
    };
  }
  if (form.condition_type === "server_expiry") {
    return {
      type: "server_expiry",
      agent_id: form.agent_id.trim(),
      days_before: Number(form.server_expiry_days_before),
    };
  }
  if (form.condition_type === "server_traffic_quota") {
    return {
      type: "server_traffic_quota",
      agent_id: form.agent_id.trim(),
      percent: Number(form.traffic_quota_percent),
      direction: form.traffic_quota_direction,
    };
  }
  if (form.condition_type === "service_latency") {
    return {
      type: "service_latency",
      service_id: form.service_id.trim(),
      max_latency_ms: Number(form.max_latency_ms),
    };
  }
  if (form.condition_type === "certificate_expiry") {
    return {
      type: "certificate_expiry",
      service_id: form.service_id.trim(),
      days_before: Number(form.cert_days_before),
    };
  }
  return {
    type: "server_resource",
    agent_id: form.agent_id.trim(),
    resource: form.resource,
    operator: form.operator,
    threshold: Number(form.threshold),
    duration_seconds: Number(form.duration_seconds),
  };
}

function splitIds(value: string): string[] {
  return value
    .split(/[\s,]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function formatConditions(copy: Translations, conditions?: JsonObject[] | undefined): string {
  if (!conditions || conditions.length === 0) return "-";
  const t = copy.alertsPage;
  return conditions
    .map((condition) => {
      const type = String(condition["type"] || "condition");
      if (type === "server_resource") {
        return `${String(condition["agent_id"] ?? "")}:${String(condition["resource"] ?? "")} ${String(condition["operator"] ?? "")} ${String(condition["threshold"] ?? "")}`;
      }
      if (type === "server_offline") {
        return `${String(condition["agent_id"] ?? "")} ${t.conditionOffline} ${String(condition["offline_seconds"] ?? "")}s`;
      }
      if (type === "server_expiry") {
        return `${String(condition["agent_id"] ?? "")} ${String(condition["days_before"] ?? "")} ${t.conditionExpiryDays}`;
      }
      if (type === "server_traffic_quota") {
        return `${String(condition["agent_id"] ?? "")} ${t.conditionTraffic} ${String(condition["direction"] || "total")} >= ${String(condition["percent"] ?? "")}%`;
      }
      if (type === "service_down") {
        return `${String(condition["service_id"] ?? "")} ${t.conditionServiceDown} x${String(condition["consecutive_failures"] ?? "")}`;
      }
      if (type === "service_latency") {
        return `${String(condition["service_id"] ?? "")} ${t.conditionLatency} > ${String(condition["max_latency_ms"] ?? "")}ms`;
      }
      if (type === "certificate_expiry") {
        return `${String(condition["service_id"] ?? "")} ${t.conditionCert} ${String(condition["days_before"] ?? "")} ${t.conditionExpiryDays}`;
      }
      return type;
    })
    .join(", ");
}

function formatPayload(copy: Translations, payload: unknown): string {
  if (!payload) return copy.alertsPage.payloadEvent;
  if (typeof payload === "string") return payload;
  try {
    return JSON.stringify(payload);
  } catch {
    return copy.alertsPage.payloadEvent;
  }
}

function formatHeaders(headers?: Record<string, string> | undefined): string {
  if (!headers || Object.keys(headers).length === 0) return "";
  return JSON.stringify(headers, null, 2);
}

function isRedactedValue(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  return trimmed === redactedSecret || trimmed.includes(redactedSecret);
}
