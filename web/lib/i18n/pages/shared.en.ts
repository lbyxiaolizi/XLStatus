// English half of the `shared` i18n namespace, split out so the default zh-CN
// bundle never ships it. `lib/i18n/en.ts` pulls these in through a dynamic
// import that only runs when the user selects en-US, so no `typeof` annotation
// is used here (it would drag the zh object back into the default chunk);
// shape is enforced by the `Translations` annotation on `enUS`.

export const dialogsEn = {
  pleaseConfirm: "Please confirm",
  confirm: "Confirm",
  ok: "OK",
  pleaseInput: "Please enter",
  twoFactor: "Two-factor",
  totpPrompt: "Enter the 6-digit TOTP code from your authenticator app",
  totpInvalid: "Enter a 6-digit numeric code",
};

export const errorsPageEn = {
  pageError: "Page error",
  loadFailed: "Failed to load page",
  unexpected: "An unexpected error occurred while rendering.",
  retry: "Retry",
  backToStatus: "Back to status",
  appError: "Application error",
  unexpectedShort: "An unexpected error occurred.",
  reload: "Reload",
};
