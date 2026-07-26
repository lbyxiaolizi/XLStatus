// Per-page i18n namespace for the WorldServerMap component.
// `zh` is the source of truth; the en half lives in `worldMap.en.ts` and is
// shape-checked by the `Translations` annotation on `enUS` in `lib/i18n/en.ts`.
// Interpolated strings use {placeholder} tokens that the component fills in with
// String.prototype.replace.

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
