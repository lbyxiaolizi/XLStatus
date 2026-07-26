// English half of the `terminal` i18n namespace, split out so the default zh-CN
// bundle never ships it. `lib/i18n/en.ts` pulls these in through a dynamic
// import that only runs when the user selects en-US, so no `typeof` annotation
// is used here (it would drag the zh object back into the default chunk);
// shape is enforced by the `Translations` annotation on `enUS`.

export const terminalPageEn = {
  selectAgentPrompt: "Select an agent and open a terminal session.",
  selectAgentBeforeOpen: "Select an agent before opening the terminal.",
  totpInvalid: "Enter a 6-digit TOTP code.",
  missingSessionId: "Terminal session response is missing a session id.",
  opening: "Opening {name} ({cols}x{rows})...",
  connected: "Connected to session {id}.",
  wsError: "WebSocket error.",
  terminalClosed: "Terminal closed.",
  serverClosed: "Server closed the terminal.",
  terminalError: "Terminal error.",
  resized: "Resized to {cols}x{rows}.",
  eyebrow: "Remote shell",
  title: "Terminal",
  detail: "Open an agent terminal session over the backend WebSocket.",
  fieldAgent: "Agent",
  agentUnknownStatus: "Unknown",
  fieldCols: "Columns",
  fieldRows: "Rows",
  openSession: "Open session",
  resize: "Resize",
  close: "Close",
  sessionLabel: "Session {id}",
  noOutput: "No terminal output yet",
  inputPlaceholderOpen: "Type a command and press Enter",
  inputPlaceholderClosed: "Open the terminal first",
  send: "Send",
  statusIdle: "Idle",
  statusConnecting: "Connecting",
  statusOpen: "Connected",
  statusClosed: "Closed",
  statusError: "Error",
};
