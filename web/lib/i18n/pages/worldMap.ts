// Per-page i18n namespace for the WorldServerMap component.
// `zh` is the source of truth; `en` must mirror its shape (enforced by the
// `typeof` annotation). Interpolated strings use {placeholder} tokens that the
// component fills in with String.prototype.replace.

export const worldMap = {
  serverUnit: "服务器",
  regionsSummary: "{regions} 个已识别地区 / {servers} 台{detail}",
  unmatchedCount: "{count} 未识别",
  allIdentified: "全部识别",
  serversCount: "{count} 台",
  moreServers: "+{count} 台",
  moreRegions: "另有 {count} 个地区",
  emptyTitle: "暂无可识别地区",
  emptyDetail: "GeoIP 或手动位置字段可用后会点亮地图。",
  unmatchedTitle: "未识别",
  tooltipSummary: "{servers} 台服务器 · {source}",
  sourceManual: "手动坐标",
  sourceCountry: "国家/地区",
};

export const worldMapEn: typeof worldMap = {
  serverUnit: "servers",
  regionsSummary: "{regions} regions identified / {servers} {detail}",
  unmatchedCount: "{count} unidentified",
  allIdentified: "All identified",
  serversCount: "{count}",
  moreServers: "+{count}",
  moreRegions: "{count} more regions",
  emptyTitle: "No identifiable regions yet",
  emptyDetail: "The map lights up once GeoIP or manual location fields are available.",
  unmatchedTitle: "Unidentified",
  tooltipSummary: "{servers} servers · {source}",
  sourceManual: "Manual coordinates",
  sourceCountry: "Country/region",
};
