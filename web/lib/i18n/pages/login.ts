// Per-page i18n namespace for the login + OAuth-callback screens.
// `zh` is the source of truth; `en` must mirror its shape (enforced by the
// `typeof` annotation). Interpolated strings use {placeholder} tokens that the
// page fills in with String.prototype.replace.

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
  // OAuth callback
  oauthCompleting: "正在完成 OAuth 登录...",
  oauthFailed: "OAuth 登录失败。",
  backToLogin: "返回登录",
};

export const loginPageEn: typeof loginPage = {
  backToStatus: "Back to status",
  adminEntry: "Admin entry",
  tagline: "Sign in to manage servers, services, tasks, and remote operations.",
  username: "Username",
  password: "Password",
  passwordPlaceholder: "Enter password",
  totpLabel: "Two-factor code",
  loggingIn: "Signing in...",
  verifyAndLogin: "Verify and sign in",
  loginWith: "Sign in with {provider}",
  loginFailed: "Login failed.",
  oauthCompleting: "Completing OAuth sign-in...",
  oauthFailed: "OAuth sign-in failed.",
  backToLogin: "Back to login",
};
