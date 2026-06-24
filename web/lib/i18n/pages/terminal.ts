// Per-page i18n namespace for the remote terminal screen.
// `zh` is the source of truth; `en` must mirror its shape (enforced by the
// `typeof` annotation). Interpolated strings use {placeholder} tokens that the
// page fills in with String.prototype.replace.

export const terminalPage = {
  // Initial line / errors
  selectAgentPrompt: "请选择 Agent 并打开终端会话。",
  selectAgentBeforeOpen: "打开终端前请选择 Agent。",
  totpInvalid: "请输入 6 位 TOTP 验证码。",
  missingSessionId: "终端会话响应缺少 session id。",
  // Terminal lifecycle lines
  opening: "正在打开 {name} ({cols}x{rows})...",
  connected: "已连接到会话 {id}。",
  wsError: "WebSocket 错误。",
  terminalClosed: "终端已关闭。",
  serverClosed: "服务端已关闭终端。",
  terminalError: "终端错误。",
  resized: "已调整为 {cols}x{rows}。",
  // Page header
  eyebrow: "远程 Shell",
  title: "终端",
  detail: "通过后端 WebSocket 建立 agent 终端会话。",
  // Form
  fieldAgent: "Agent",
  agentUnknownStatus: "未知",
  fieldCols: "列数",
  fieldRows: "行数",
  openSession: "打开会话",
  resize: "调整大小",
  close: "关闭",
  sessionLabel: "会话 {id}",
  // Output panel
  noOutput: "暂无终端输出",
  inputPlaceholderOpen: "输入命令并按 Enter",
  inputPlaceholderClosed: "请先打开终端",
  send: "发送",
  // Status labels
  statusIdle: "空闲",
  statusConnecting: "连接中",
  statusOpen: "已连接",
  statusClosed: "已关闭",
  statusError: "错误",
};

export const terminalPageEn: typeof terminalPage = {
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
