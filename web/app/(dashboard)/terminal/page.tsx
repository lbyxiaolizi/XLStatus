"use client";

import { FormEvent, KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  BrutalCard,
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
  PageHeader,
  PageShell,
  StatusBadge,
  buttonClass,
  compactId,
  inputClass,
  responseError,
  selectClass,
} from "@/app/components/M7Primitives";
import { apiClient, buildWebSocketUrl, type TotpStatusResponse } from "@/lib/api";
import { useDialogs } from "@/app/components/Dialogs";
import { useServerData } from "@/app/components/ServerDataProvider";
import { useI18n } from "@/lib/use-i18n";

type TerminalStatus = "idle" | "connecting" | "open" | "closed" | "error";

interface TerminalLine {
  id: number;
  direction: "system" | "input" | "output" | "error";
  text: string;
}

const QUICK_OPS_SNIPPETS: readonly { id: string; label: string; command: string }[] = [
  { id: "uname", label: "uname -a", command: "uname -a" },
  { id: "df", label: "df -h", command: "df -h" },
  { id: "uptime", label: "uptime", command: "uptime" },
  { id: "ps", label: "ps aux | head -n 20", command: "ps aux | head -n 20" },
  { id: "clear", label: "clear", command: "clear" },
];

export default function TerminalPage() {
  const dialogs = useDialogs();
  const { t: copy, locale } = useI18n();
  const isZh = locale === "zh-CN";
  const {
    servers,
    loading,
    error: serverLoadError,
    revalidate: revalidateServers,
  } = useServerData();
  const [agentId, setAgentId] = useState("");
  const [cols, setCols] = useState(100);
  const [rows, setRows] = useState(30);
  const [input, setInput] = useState("");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [status, setStatus] = useState<TerminalStatus>("idle");
  const [opening, setOpening] = useState(false);
  const [tallTerminal, setTallTerminal] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [totpStatus, setTotpStatus] = useState<TotpStatusResponse | null>(null);
  const [lines, setLines] = useState<TerminalLine[]>([
    { id: 1, direction: "system", text: copy.terminalPage.selectAgentPrompt },
  ]);
  const wsRef = useRef<WebSocket | null>(null);
  const lineIdRef = useRef(2);
  const outputRef = useRef<HTMLDivElement | null>(null);
  const pendingLinesRef = useRef<TerminalLine[]>([]);
  const flushRafRef = useRef<number | null>(null);

  const selectedAgent = useMemo(() => servers.find((server) => server.id === agentId), [agentId, servers]);
  const onlineCount = useMemo(
    () => servers.filter((server) => String(server.status || "").toLowerCase() === "online").length,
    [servers],
  );

  const flushLines = useCallback(() => {
    if (flushRafRef.current !== null) {
      window.cancelAnimationFrame(flushRafRef.current);
      flushRafRef.current = null;
    }
    const pending = pendingLinesRef.current;
    if (pending.length === 0) return;
    pendingLinesRef.current = [];
    setLines((prev) => [...prev, ...pending].slice(-400));
  }, []);

  const appendLine = useCallback(
    (direction: TerminalLine["direction"], text: string) => {
      pendingLinesRef.current.push({ id: lineIdRef.current++, direction, text });
      if (flushRafRef.current === null) {
        flushRafRef.current = window.requestAnimationFrame(flushLines);
      }
    },
    [flushLines],
  );

  const closeSocket = useCallback(() => {
    wsRef.current?.close();
    wsRef.current = null;
    flushLines();
  }, [flushLines]);

  useEffect(() => {
    void revalidateServers();
  }, [revalidateServers]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- picker default derived from the shared server list, kept out of render
    setAgentId((current) => current || servers[0]?.id || "");
  }, [servers]);

  useEffect(() => {
    outputRef.current?.scrollTo({ top: outputRef.current.scrollHeight, behavior: "auto" });
  }, [lines]);

  useEffect(
    () => () => {
      wsRef.current?.close();
      wsRef.current = null;
      if (flushRafRef.current !== null) {
        window.cancelAnimationFrame(flushRafRef.current);
        flushRafRef.current = null;
      }
      pendingLinesRef.current = [];
    },
    [],
  );

  async function openTerminal(event?: FormEvent) {
    event?.preventDefault();
    if (!agentId) {
      setError(copy.terminalPage.selectAgentBeforeOpen);
      return;
    }

    closeSocket();
    setError(null);
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setOpening(true);
    setStatus("connecting");
    setLines([
      {
        id: lineIdRef.current++,
        direction: "system",
        text: copy.terminalPage.opening
          .replace("{name}", String(selectedAgent?.name || compactId(agentId)))
          .replace("{cols}", String(cols))
          .replace("{rows}", String(rows)),
      },
    ]);

    const response = await apiClient.createTerminalSession(agentId, cols, rows, totpCode);
    if (!response.success) {
      setOpening(false);
      setStatus("error");
      setError(responseError(response));
      appendLine("error", responseError(response));
      return;
    }

    const nextSessionId = response.data?.session_id || response.data?.id;
    if (!nextSessionId) {
      setOpening(false);
      setStatus("error");
      setError(copy.terminalPage.missingSessionId);
      return;
    }

    setSessionId(nextSessionId);
    connectTerminal(nextSessionId);
  }

  async function sensitiveTotpCode(): Promise<string | undefined | null> {
    let enabled = totpStatus?.enabled;
    if (totpStatus === null) {
      const response = await apiClient.getTotpStatus();
      if (!response.success || !response.data) {
        setError(responseError(response));
        return null;
      }
      setTotpStatus(response.data);
      enabled = response.data.enabled;
    }
    if (!enabled) return undefined;
    const code = await dialogs.totp();
    if (code === null) return null;
    const trimmed = code.trim();
    if (!/^\d{6}$/.test(trimmed)) {
      setError(copy.terminalPage.totpInvalid);
      return null;
    }
    return trimmed;
  }

  function connectTerminal(nextSessionId: string) {
    const ws = new WebSocket(buildTerminalWsUrl(nextSessionId));
    wsRef.current = ws;

    ws.onopen = () => {
      setOpening(false);
      setStatus("open");
      appendLine("system", copy.terminalPage.connected.replace("{id}", String(compactId(nextSessionId))));
      sendFrame({ type: "terminal.resize", cols, rows });
    };
    ws.onmessage = (event) => void handleTerminalMessage(event.data as string | Blob | ArrayBuffer);
    ws.onerror = () => {
      setOpening(false);
      setStatus("error");
      appendLine("error", copy.terminalPage.wsError);
    };
    ws.onclose = () => {
      setOpening(false);
      setStatus((current) => (current === "error" ? "error" : "closed"));
      appendLine("system", copy.terminalPage.terminalClosed);
      wsRef.current = null;
    };
  }

  async function handleTerminalMessage(data: string | Blob | ArrayBuffer) {
    if (typeof data !== "string") {
      const text = data instanceof Blob ? await data.text() : new TextDecoder().decode(data);
      appendLine("output", text);
      return;
    }

    try {
      const msg = JSON.parse(data) as Record<string, unknown>;
      const type = String(msg["type"] || "");
      if (type === "terminal.output" || type === "output") {
        appendLine("output", String(msg["data"] ?? msg["output"] ?? ""));
        return;
      }
      if (type === "terminal.closed" || type === "closed") {
        appendLine("system", String(msg["reason"] || copy.terminalPage.serverClosed));
        closeSocket();
        return;
      }
      if (type === "terminal.error" || type === "error") {
        appendLine("error", String(msg["error"] || msg["message"] || copy.terminalPage.terminalError));
        return;
      }
      appendLine("system", data);
    } catch {
      appendLine("output", data);
    }
  }

  function sendInput() {
    const value = input;
    if (!value || status !== "open") return;
    sendFrame({ type: "terminal.input", data: `${value}\n` });
    appendLine("input", `$ ${value}`);
    setInput("");
  }

  function sendQuickSnippet(command: string) {
    if (status === "open") {
      if (command === "clear") {
        pendingLinesRef.current = [];
        setLines([]);
      } else {
        appendLine("input", `$ ${command}`);
      }
      sendFrame({ type: "terminal.input", data: `${command}\n` });
      return;
    }
    setInput(command);
  }

  function resizeTerminal() {
    if (status !== "open") return;
    sendFrame({ type: "terminal.resize", cols, rows });
    appendLine(
      "system",
      copy.terminalPage.resized.replace("{cols}", String(cols)).replace("{rows}", String(rows)),
    );
  }

  function sendFrame(frame: Record<string, unknown>) {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(frame));
    }
  }

  function onInputKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      event.preventDefault();
      sendInput();
    }
  }

  const telemetryStatusText =
    status === "open"
      ? "Connected"
      : status === "connecting"
        ? "Connecting"
        : "Disconnected";

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.terminalPage.eyebrow}
          title={copy.terminalPage.title}
          detail={copy.terminalPage.detail}
          actions={
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge
                tone={
                  status === "open"
                    ? "green"
                    : status === "connecting"
                      ? "yellow"
                      : status === "error"
                        ? "red"
                        : "gray"
                }
              >
                {terminalStatusLabel(status, copy)} ({telemetryStatusText})
              </StatusBadge>
            </div>
          }
        />
        <InlineError message={error ?? serverLoadError} />

        {/* Node Quick-Switcher & Connection Telemetry Bar */}
        <BrutalCard className="mt-4 border-[var(--border-color)] bg-gradient-to-br from-[var(--bg-card)] to-[var(--bg-subtle)] p-4">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-color)] pb-3">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-xs font-semibold tracking-tight text-[var(--text-main)]">
                {isZh ? "节点快速切换 / Node Quick-Switcher" : "Node Quick-Switcher"}
              </span>
              <span className="rounded-full bg-emerald-500/10 px-2 py-0.5 font-mono text-[11px] font-medium text-emerald-600 dark:text-emerald-400">
                {onlineCount}/{servers.length} {isZh ? "在线" : "online"}
              </span>
            </div>

            {/* Connection Telemetry Indicators */}
            <div className="flex flex-wrap items-center gap-2 text-xs">
              <span className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1 font-mono text-xs text-[var(--text-main)]">
                <span
                  className={`h-2 w-2 rounded-full ${
                    status === "open"
                      ? "bg-emerald-500"
                      : status === "connecting"
                        ? "bg-amber-500 animate-pulse"
                        : status === "error"
                          ? "bg-rose-500"
                          : "bg-zinc-400"
                  }`}
                />
                <span>{telemetryStatusText}</span>
              </span>
              <span className="inline-flex items-center rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1 font-mono text-xs text-[var(--text-muted)]">
                {cols} × {rows}
              </span>
              {sessionId ? (
                <span className="inline-flex items-center rounded-lg border border-fuchsia-500/30 bg-fuchsia-500/10 px-2.5 py-1 font-mono text-xs text-fuchsia-600 dark:text-fuchsia-300">
                  {copy.terminalPage.sessionLabel.replace("{id}", String(compactId(sessionId)))}
                </span>
              ) : (
                <span className="inline-flex items-center rounded-lg border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-1 font-mono text-xs text-[var(--text-muted)]">
                  {isZh ? "无活跃会话 / No Session" : "No Active Session"}
                </span>
              )}
            </div>
          </div>

          <div className="mt-3 flex flex-wrap gap-2">
            {servers.length === 0 ? (
              <span className="text-xs text-[var(--text-muted)]">
                {isZh ? "暂无节点" : "No nodes available"}
              </span>
            ) : (
              servers.map((server) => {
                const isSelected = server.id === agentId;
                const isOnline = String(server.status || "").toLowerCase() === "online";
                return (
                  <button
                    key={server.id}
                    type="button"
                    onClick={() => setAgentId(server.id)}
                    aria-pressed={isSelected}
                    className={`inline-flex items-center gap-2 rounded-full border px-3 py-1 text-xs font-medium transition-all cursor-pointer ${
                      isSelected
                        ? "border-emerald-500/60 bg-emerald-500/15 text-[var(--text-main)] ring-1 ring-emerald-500/30"
                        : "border-[var(--border-color)] bg-[var(--bg-card)] text-[var(--text-muted)] hover:border-[var(--border-strong)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    <span
                      className={`h-2 w-2 rounded-full ${
                        isOnline ? "bg-emerald-500" : "bg-zinc-400"
                      }`}
                    />
                    <span>{server.name}</span>
                  </button>
                );
              })
            )}
          </div>
        </BrutalCard>

        <div className="mt-5 grid gap-6 lg:grid-cols-[340px_minmax(0,1fr)]">
          <BrutalCard>
            <form onSubmit={openTerminal} className="space-y-4">
              <Field label={copy.terminalPage.fieldAgent}>
                <select className={selectClass} value={agentId} onChange={(e) => setAgentId(e.target.value)}>
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>
                      {server.name} ({server.status || copy.terminalPage.agentUnknownStatus})
                    </option>
                  ))}
                </select>
              </Field>
              <div className="grid grid-cols-2 gap-3">
                <Field label={copy.terminalPage.fieldCols}>
                  <input
                    className={inputClass}
                    value={cols}
                    onChange={(e) => setCols(Number(e.target.value) || 100)}
                  />
                </Field>
                <Field label={copy.terminalPage.fieldRows}>
                  <input
                    className={inputClass}
                    value={rows}
                    onChange={(e) => setRows(Number(e.target.value) || 30)}
                  />
                </Field>
              </div>
              <div className="flex flex-wrap gap-2">
                <button disabled={opening || loading} className={buttonClass("primary")}>
                  {copy.terminalPage.openSession}
                </button>
                <button type="button" onClick={resizeTerminal} className={buttonClass("secondary")}>
                  {copy.terminalPage.resize}
                </button>
                <button type="button" onClick={closeSocket} className={buttonClass("danger")}>
                  {copy.terminalPage.close}
                </button>
              </div>
              {sessionId ? (
                <InlineNotice tone="pink">
                  {copy.terminalPage.sessionLabel.replace("{id}", String(compactId(sessionId)))}
                </InlineNotice>
              ) : null}
            </form>
          </BrutalCard>

          <BrutalCard className="overflow-hidden p-0">
            {/* Terminal Window Header */}
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-zinc-800 bg-zinc-900 px-4 py-2.5">
              <div className="flex items-center gap-1.5">
                <span className="h-2.5 w-2.5 rounded-full bg-rose-500/80" />
                <span className="h-2.5 w-2.5 rounded-full bg-amber-500/80" />
                <span className="h-2.5 w-2.5 rounded-full bg-emerald-500/80" />
              </div>
              <span className="font-mono text-xs text-zinc-400">
                {selectedAgent?.name || "shell"} — {cols}×{rows}
              </span>
              <button
                type="button"
                onClick={() => setTallTerminal((prev) => !prev)}
                className="inline-flex items-center gap-1 rounded border border-zinc-700 bg-zinc-800 px-2.5 py-1 font-mono text-[11px] font-medium text-zinc-200 transition hover:border-emerald-500/50 hover:text-emerald-300 cursor-pointer"
              >
                {tallTerminal
                  ? isZh
                    ? "标准高度 (480px)"
                    : "Standard (480px)"
                  : isZh
                    ? "加高视图 (680px)"
                    : "Expand (680px)"}
              </button>
            </div>

            {/* Quick Ops Snippet Bar right above the terminal canvas */}
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-zinc-800/90 bg-zinc-900/60 px-4 py-2">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="mr-1 font-mono text-[11px] text-zinc-400">
                  {isZh ? "快捷指令 / Snippets:" : "Quick Snippets:"}
                </span>
                {QUICK_OPS_SNIPPETS.map((snippet) => (
                  <button
                    key={snippet.id}
                    type="button"
                    onClick={() => sendQuickSnippet(snippet.command)}
                    className="rounded border border-zinc-700/80 bg-zinc-800/80 px-2 py-0.5 font-mono text-[11px] text-emerald-300 transition hover:border-emerald-500/60 hover:bg-emerald-500/15 cursor-pointer"
                  >
                    {snippet.label}
                  </button>
                ))}
              </div>
              <span className="font-mono text-[11px] text-zinc-500">
                {tallTerminal ? "680px" : "480px"}
              </span>
            </div>

            {/* Terminal Output Canvas */}
            <div
              ref={outputRef}
              className={`${
                tallTerminal ? "h-[680px]" : "h-[480px]"
              } overflow-auto bg-zinc-950 p-4 font-mono text-xs leading-relaxed text-emerald-400 transition-all duration-200`}
            >
              {lines.length === 0 ? <EmptyState title={copy.terminalPage.noOutput} /> : null}
              {lines.map((line) => (
                <div
                  key={line.id}
                  className={
                    line.direction === "error"
                      ? "text-rose-400"
                      : line.direction === "input"
                        ? "text-sky-400"
                        : line.direction === "system"
                          ? "text-zinc-400"
                          : ""
                  }
                >
                  {line.text}
                </div>
              ))}
            </div>
            <div className="flex border-t border-zinc-800 bg-zinc-900">
              <span className="flex items-center pl-4 font-mono text-xs text-emerald-500">$</span>
              <input
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={onInputKeyDown}
                disabled={status !== "open"}
                className="min-w-0 flex-1 bg-transparent px-3 py-3 font-mono text-xs text-zinc-100 outline-none placeholder:text-zinc-500"
                placeholder={
                  status === "open"
                    ? copy.terminalPage.inputPlaceholderOpen
                    : copy.terminalPage.inputPlaceholderClosed
                }
              />
              <button
                type="button"
                onClick={sendInput}
                disabled={status !== "open"}
                className="border-l border-zinc-800 bg-[var(--accent-color)] px-5 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
              >
                {copy.terminalPage.send}
              </button>
            </div>
          </BrutalCard>
        </div>
      </PageShell>
      {dialogs.element}
    </div>
  );
}

function buildTerminalWsUrl(sessionId: string): string {
  return buildWebSocketUrl(`/ws/terminal/${encodeURIComponent(sessionId)}`);
}

function terminalStatusLabel(status: TerminalStatus, copy: ReturnType<typeof useI18n>["t"]): string {
  const labels: Record<TerminalStatus, string> = {
    idle: copy.terminalPage.statusIdle,
    connecting: copy.terminalPage.statusConnecting,
    open: copy.terminalPage.statusOpen,
    closed: copy.terminalPage.statusClosed,
    error: copy.terminalPage.statusError,
  };
  return labels[status];
}
