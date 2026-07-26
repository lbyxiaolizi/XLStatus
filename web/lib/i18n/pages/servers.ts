// Per-page i18n namespace for the servers list screen.
// `zh` is the source of truth; the en half lives in `servers.en.ts` and is
// shape-checked by the `Translations` annotation on `enUS` in `lib/i18n/en.ts`.
// Interpolated strings use {placeholder} tokens that the page fills in with
// String.prototype.replace.

export const serversPage = {
  // Page header
  title: "服务器",
  detail: "接入的 Agent、实时主机状态和远程运维入口。",
  refresh: "刷新",

  // Toolbar: search / filters / sort / view
  searchPlaceholder: "搜索服务器",
  filterAll: "全部",
  filterOnline: "在线",
  filterOffline: "离线",
  filterOther: "其他",
  groupFilterAll: "全部分组",
  sortDefault: "默认排序",
  sortName: "名称",
  sortStatus: "状态",
  sortMemory: "内存",
  sortLoad: "负载",
  sortUptime: "运行时间",
  sortUpload: "上传速率",
  sortDownload: "下载速率",
  sortTotalUpload: "累计上传",
  sortTotalDownload: "累计下载",
  sortAsc: "升序",
  sortDesc: "降序",
  selectPage: "选择本页",
  deselectPage: "取消本页",
  viewCards: "卡片",
  viewCompact: "紧凑",
  toggleMap: "地图",
  toggleServices: "服务状态",

  // Server groups manager
  serverGroupsBadge: "{count} 组",
  serverGroupsSelectedCount: "{count} 台",
  serverGroupsExpand: "展开管理",
  refreshGroups: "刷新组",
  newServerGroupPlaceholder: "新服务器分组",
  createGroup: "创建组",
  selectServerGroupOption: "选择服务器分组",
  groupNamePlaceholder: "分组名称",
  groupOrderPlaceholder: "排序",
  groupColorAriaLabel: "分组颜色",
  saveGroup: "保存组",
  deleteGroup: "删除组",

  // Selection / batch toolbar
  selectedCount: "已选择 {count} 台",
  clearSelection: "清空选择",
  batchTagsPlaceholder: "分组标签，逗号分隔",
  setTags: "设置分组",
  addTags: "追加分组",
  removeTags: "移除分组",
  showOnDashboard: "显示状态页",
  hideOnDashboard: "隐藏状态页",
  ownerUserIdPlaceholder: "目标用户 ID",
  transferOwner: "转移所有者",
  addToGroup: "加入组",
  moveToGroup: "移动到组",
  deleteSelectedServers: "删除选中服务器",

  // Owner transfers section
  ownerTransfersTitle: "所有权转移",
  ownerTransfersBadge: "{count} 条",
  ownerTransfersExpand: "展开处理",
  ownerTransfersLoading: "加载中",
  ownerTransfersRefresh: "刷新记录",
  ownerTransfersEmptyTitle: "暂无转移记录",
  ownerTransfersEmptyDetail: "批量转移服务器所有者后会显示在这里。",
  retry: "重试",
  cancel: "取消",
  completed: "完成",

  // World map
  mapTitle: "服务器地图",
  mapAriaLabel: "服务器国家和地区分布地图",

  // Service tracker panel
  servicesTitle: "服务状态",
  servicesLoading: "加载中",
  servicesItemCount: "{count} 项",
  servicesLoadingCard: "正在加载服务状态...",
  servicesEmptyTitle: "暂无服务状态",
  servicesEmptyDetail: "创建服务监控后会显示最近状态。",
  servicesTrafficSummary: "周期流量统计：暂无数据",
  serviceDays30Ago: "30 天前",
  serviceDaysToday: "今天",

  // Server list states
  serversEmptyTitle: "暂无服务器",
  serversEmptyDetail: "Agent 连接后会显示在这里。",

  // Summary grid
  summaryServers: "服务器",
  summaryUpload: "上传",
  summaryDownload: "下载",
  summaryStatus: "状态",
  summaryOnlineOffline: "{online} 在线 / {offline} 离线",
  summaryTotal: "累计 {total}",
  summaryPartialOffline: "部分离线",
  summaryRunning: "运行中",
  summaryWsLive: "ws 实时更新",
  summaryWsWaiting: "ws 等待数据",

  // Server card
  cardSelect: "选择",
  cardSelectAria: "选择 {name}",
  cardOpenAria: "打开服务器 {name}",
  cardDetail: "详情",
  metaRemark: "备注",
  metaProvider: "供应商",
  metaRegion: "地区",
  metaPlan: "套餐",
  metaExpires: "到期",
  metaRenewal: "续费",
  metricCpu: "CPU",
  metricMemory: "内存",
  metricLoad: "负载",
  metricUptime: "运行时间",
  metricUptimeShort: "运行",
  metricUpload: "上传",
  metricDownload: "下载",
  metricTotalUpload: "累计上传",
  metricTotalDownload: "累计下载",
  metricStatus: "状态",
  metricTraffic: "流量",

  // Connection states
  connConnecting: "连接中",
  connOpen: "已连接",
  connClosed: "已关闭",
  connError: "错误",

  // Server status labels
  statusOnline: "在线",
  statusOffline: "离线",
  statusRevoked: "已撤销",
  statusDown: "异常",
  statusDegraded: "降级",

  // Notices / errors / confirmations
  batchActionDone: "批量操作完成：{updated} 成功 / {failed} 失败。",
  groupNameRequired: "请填写服务器分组名称。",
  groupCreated: "服务器分组 {name} 已创建。",
  selectGroupToEdit: "请选择要编辑的服务器分组。",
  orderMustBeNumber: "排序必须是数字。",
  groupUpdated: "服务器分组 {name} 已更新。",
  selectTargetGroup: "请选择目标服务器分组。",
  selectServersFirst: "请先选择服务器。",
  addedToGroup: "已把选中服务器加入分组。",
  selectGroupToDelete: "请选择要删除的服务器分组。",
  deleteGroupConfirm: "确定删除服务器分组 {name}？",
  groupDeleted: "服务器分组已删除。",
  deleteServersConfirm: "确定删除选中的 {count} 台服务器？该操作会移除 Agent 记录及关联分组。",
  tagRequired: "请填写至少一个分组标签。",
  ownerUserIdRequired: "请填写目标用户 ID。",
  transferRetried: "转移记录 {id} 已重试。",
  cancelTransferConfirm: "确定取消转移记录 {id}？",
  transferCancelled: "转移记录 {id} 已取消。",
};
