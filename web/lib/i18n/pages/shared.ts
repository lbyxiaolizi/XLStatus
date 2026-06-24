// Per-page i18n namespaces shared across multiple dashboard components
// (in-app dialogs, route/global error boundaries). `zh` is the source of
// truth; the `en` variants mirror its shape (enforced by the `typeof`
// annotation).

export const dialogs = {
  pleaseConfirm: "请确认",
  confirm: "确认",
  ok: "确定",
  pleaseInput: "请输入",
  twoFactor: "二次验证",
  totpPrompt: "请输入身份验证器中的 6 位 TOTP 验证码",
  totpInvalid: "请输入 6 位数字验证码",
};
export const dialogsEn: typeof dialogs = {
  pleaseConfirm: "Please confirm",
  confirm: "Confirm",
  ok: "OK",
  pleaseInput: "Please enter",
  twoFactor: "Two-factor",
  totpPrompt: "Enter the 6-digit TOTP code from your authenticator app",
  totpInvalid: "Enter a 6-digit numeric code",
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
export const errorsPageEn: typeof errorsPage = {
  pageError: "Page error",
  loadFailed: "Failed to load page",
  unexpected: "An unexpected error occurred while rendering.",
  retry: "Retry",
  backToStatus: "Back to status",
  appError: "Application error",
  unexpectedShort: "An unexpected error occurred.",
  reload: "Reload",
};
