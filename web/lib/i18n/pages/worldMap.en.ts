// English half of the `worldMap` i18n namespace, split out so the default zh-CN
// bundle never ships it. `lib/i18n/en.ts` pulls these in through a dynamic
// import that only runs when the user selects en-US, so no `typeof` annotation
// is used here (it would drag the zh object back into the default chunk);
// shape is enforced by the `Translations` annotation on `enUS`.

export const worldMapEn = {
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
