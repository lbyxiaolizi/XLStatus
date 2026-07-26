// English half of the `login` i18n namespace, split out so the default zh-CN
// bundle never ships it. `lib/i18n/en.ts` pulls these in through a dynamic
// import that only runs when the user selects en-US, so no `typeof` annotation
// is used here (it would drag the zh object back into the default chunk);
// shape is enforced by the `Translations` annotation on `enUS`.

export const loginPageEn = {
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
  sessionExpired: "Your session has expired, please sign in again.",
  errInvalidCredentials: "Incorrect username or password.",
  errInvalidTotp: "Incorrect two-factor code, please try again.",
  errIpBanned: "This IP is temporarily blocked, please try again later.",
  errLoginRejected: "The sign-in request was rejected, refresh the page and try again.",
  errServerUnavailable: "The server is temporarily unavailable, please try again later.",
  oauthCompleting: "Completing OAuth sign-in...",
  oauthFailed: "OAuth sign-in failed.",
  backToLogin: "Back to login",
};
