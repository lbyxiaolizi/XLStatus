// Per-page i18n namespace for the DDNS screen.
// `zh` is the source of truth; the en half lives in `ddns.en.ts` and is
// shape-checked by the `Translations` annotation on `enUS` in `lib/i18n/en.ts`.
// Interpolated strings use {placeholder} tokens that the page fills in with
// String.prototype.replace.

export const ddnsPage = {
  // notices / errors
  configCreated: "DDNS 配置已创建。",
  configDeleted: "DDNS 配置已删除。",
  providersReloaded: "DDNS Provider 已重载。",
  checkTriggered: "DDNS 检查已触发。",
  historyLoaded: "已加载 {name} 的历史。",
  deleteConfigConfirm: "确定删除 DDNS 配置「{name}」？",
  invalidTotp: "请输入 6 位 TOTP 验证码。",
  // page header
  eyebrow: "DNS",
  title: "DDNS",
  detail: "动态 DNS 配置、重载和更新历史。",
  reload: "重载",
  checkNow: "立即检查",
  addConfig: "新增配置",
  // empty state
  emptyTitle: "暂无 DDNS 配置",
  // table columns
  colName: "名称",
  colProvider: "Provider",
  colDomain: "域名",
  colIp: "IP",
  colStatus: "状态",
  colActions: "操作",
  disabled: "停用",
  enabled: "启用",
  history: "历史",
  delete: "删除",
  // history section
  historyHeading: "历史",
  success: "成功",
  failed: "失败",
  // modal
  modalTitle: "新增 DDNS 配置",
  fieldName: "名称",
  fieldProvider: "Provider",
  fieldAgentId: "Agent ID",
  fieldDomain: "域名",
  fieldWebhookUrl: "Webhook URL",
  fieldRecordId: "Record ID",
  fieldZoneId: "Zone ID",
  fieldApiToken: "API Token",
  fieldApiKey: "API Key",
  fieldApiSecret: "API Secret",
  saveConfig: "保存配置",
};
