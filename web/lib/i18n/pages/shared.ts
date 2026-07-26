// Per-page i18n namespaces shared across multiple dashboard components
// (in-app dialogs, route/global error boundaries). `zh` is the source of
// truth; the en half lives in `shared.en.ts` and is shape-checked by the
// `Translations` annotation on `enUS` in `lib/i18n/en.ts`.

export const dialogs = {
  pleaseConfirm: "请确认",
  confirm: "确认",
  ok: "确定",
  pleaseInput: "请输入",
  twoFactor: "二次验证",
  totpPrompt: "请输入身份验证器中的 6 位 TOTP 验证码",
  totpInvalid: "请输入 6 位数字验证码",
};

export const errorsPage = {
  pageError: "页面出错",
  loadFailed: "页面加载失败",
  unexpected: "渲染过程中发生未预期的错误。",
  retry: "重试",
  backToStatus: "返回状态页",
  appError: "应用出错",
  unexpectedShort: "发生未预期的错误。",
  reload: "重新加载",
};
