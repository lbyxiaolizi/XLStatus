// Per-page i18n namespace for the login + OAuth-callback screens.
// `zh` is the source of truth; the en half lives in `login.en.ts` and is
// shape-checked by the `Translations` annotation on `enUS` in `lib/i18n/en.ts`.
// Interpolated strings use {placeholder} tokens that the page fills in with
// String.prototype.replace.

export const loginPage = {
  backToStatus: "返回状态页",
  adminEntry: "管理员入口",
  tagline: "登录后管理服务器、服务、任务和远程运维能力。",
  username: "用户名",
  password: "密码",
  passwordPlaceholder: "请输入密码",
  totpLabel: "两步验证码",
  loggingIn: "正在登录...",
  verifyAndLogin: "验证并登录",
  loginWith: "使用 {provider} 登录",
  loginFailed: "登录失败。",
  sessionExpired: "会话已过期，请重新登录。",
  errInvalidCredentials: "用户名或密码错误。",
  errInvalidTotp: "两步验证码不正确，请重试。",
  errIpBanned: "当前 IP 已被临时封禁，请稍后再试。",
  errLoginRejected: "登录请求被拒绝，请刷新页面后重试。",
  errServerUnavailable: "服务器暂时不可用，请稍后重试。",
  // OAuth callback
  oauthCompleting: "正在完成 OAuth 登录...",
  oauthFailed: "OAuth 登录失败。",
  backToLogin: "返回登录",
};
