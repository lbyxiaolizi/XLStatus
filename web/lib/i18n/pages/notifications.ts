// Per-page i18n namespace for the notifications management screen.
// `zh` is the source of truth; the en half lives in `notifications.en.ts` and is
// shape-checked by the `Translations` annotation on `enUS` in `lib/i18n/en.ts`.
// Interpolated strings use {placeholder} tokens that the page fills in with
// String.prototype.replace.

export const notificationsPage = {
  // TOTP / errors
  totpInvalid: "请输入 6 位 TOTP 验证码。",
  // Notices
  channelUpdated: "通知渠道已更新。",
  channelCreated: "通知渠道已创建。",
  groupUpdated: "通知组已更新。",
  groupCreated: "通知组已创建。",
  testSent: "测试通知已发送：{name}",
  channelDeleted: "通知渠道已删除。",
  groupDeleted: "通知组已删除。",
  memberAdded: "已加入通知组：{name}",
  memberRemoved: "已移出通知组：{name}",
  // Confirms
  deleteChannelConfirm: "确定删除通知渠道「{name}」？",
  deleteGroupConfirm: "确定删除通知组「{name}」？",
  // Page header
  title: "通知",
  detail: "通知渠道、模板和分组。",
  refresh: "刷新",
  addGroup: "新增通知组",
  addChannel: "新增渠道",
  // Channels section
  channels: "渠道",
  noChannels: "暂无通知渠道",
  colName: "名称",
  colType: "类型",
  colTls: "TLS",
  colUpdated: "更新",
  colActions: "操作",
  tlsSkip: "跳过",
  tlsVerify: "校验",
  test: "测试",
  edit: "编辑",
  delete: "删除",
  // Groups section
  groups: "通知组",
  noGroups: "暂无通知组",
  channelCountSuffix: "个渠道",
  noMembers: "暂无成员",
  removeMember: "移出",
  selectChannel: "选择渠道",
  join: "加入",
  // Notification modal
  editChannel: "编辑通知渠道",
  createChannel: "新增通知渠道",
  fieldName: "名称",
  fieldProvider: "Provider",
  keepCurrentTemplate: "保持当前模板",
  fieldUrl: "URL",
  fieldMethod: "Method",
  fieldType: "Type",
  fieldHeadersJson: "Headers JSON",
  fieldBodyTemplate: "Body Template",
  verifyTlsCert: "TLS 证书校验",
  formatMetricUnits: "格式化指标单位",
  saveChannel: "保存渠道",
  // Group modal
  editGroup: "编辑通知组",
  createGroup: "新增通知组",
  saveGroup: "保存通知组",
  // fallback provider presets
  providerGenericJson: "通用 JSON Webhook",
  providerGenericForm: "通用表单 Webhook",
  providerCustomBody: "自定义 Body",
};
