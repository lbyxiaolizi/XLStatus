// Per-page i18n namespace for the alerts screen.
// `zh` is the source of truth; the en half lives in `alerts.en.ts` and is
// shape-checked by the `Translations` annotation on `enUS` in `lib/i18n/en.ts`.
// Interpolated strings use {placeholder} tokens that the page fills in with
// String.prototype.replace.

export const alertsPage = {
  // notices / errors
  ruleCreated: "告警规则已创建。",
  ruleDeleted: "告警规则已删除。",
  deleteRuleConfirm: "确定删除告警规则「{name}」？",
  invalidTotp: "请输入 6 位 TOTP 验证码。",
  // page header
  eyebrow: "事件规则",
  title: "告警",
  detail: "资源、服务和恢复通知的规则管理。",
  addRule: "新增规则",
  // rules section
  rulesHeading: "规则",
  noRules: "暂无告警规则",
  colName: "名称",
  colCondition: "条件",
  colStatus: "状态",
  colActions: "操作",
  disabled: "停用",
  enabled: "启用",
  delete: "删除",
  // events section
  eventsHeading: "事件",
  noEvents: "暂无告警事件",
  // modal
  modalTitle: "新增告警规则",
  fieldName: "名称",
  fieldTrigger: "触发方式",
  fieldConditionType: "条件类型",
  fieldAgentId: "Agent ID",
  fieldServiceId: "服务 ID",
  fieldResource: "资源",
  fieldOperator: "操作符",
  fieldThreshold: "阈值",
  fieldDurationSeconds: "持续秒数",
  fieldOfflineSeconds: "离线秒数",
  fieldDaysBefore: "提前天数",
  fieldUsagePercent: "使用百分比",
  fieldDirection: "方向",
  fieldConsecutiveFailures: "连续失败次数",
  fieldMaxLatencyMs: "最大延迟 ms",
  fieldNotificationGroupId: "通知组 ID",
  fieldFailureTaskIds: "失败任务 IDs",
  fieldRecoveryTaskIds: "恢复任务 IDs",
  saveRule: "保存规则",
  // condition / payload formatting
  conditionOffline: "离线",
  conditionExpiryDays: "天内到期",
  conditionTraffic: "流量",
  conditionServiceDown: "异常",
  conditionLatency: "延迟",
  conditionCert: "证书",
  payloadEvent: "事件",
};
