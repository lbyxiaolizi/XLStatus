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

export default function TerminalPage() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
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

  // A chatty shell emits many frames per animation frame; buffering them means
  // one commit per frame instead of one re-render of the 400-line log per frame.
  const appendLine = useCallback((direction: TerminalLine["direction"], text: string) => {
    pendingLinesRef.current.push({ id: lineIdRef.current++, direction, text });
    if (flushRafRef.current === null) {
      flushRafRef.current = window.requestAnimationFrame(flushLines);
    }
  }, [flushLines]);

  const closeSocket = useCallback(() => {
    wsRef.current?.close();
    wsRef.current = null;
    // Drain synchronously: frames buffered right before a close (the server's
    // close reason, for one) must still reach the log.
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

  useEffect(() => () => {
    // Unmount cleanup is inlined rather than reusing closeSocket so no flush
    // (and no setState) is scheduled against a component that is going away.
    wsRef.current?.close();
    wsRef.current = null;
    if (flushRafRef.current !== null) {
      window.cancelAnimationFrame(flushRafRef.current);
      flushRafRef.current = null;
    }
    pendingLinesRef.current = [];
  }, []);

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
    setLines([{ id: lineIdRef.current++, direction: "system", text: copy.terminalPage.opening.replace("{name}", String(selectedAgent?.name || compactId(agentId))).replace("{cols}", String(cols)).replace("{rows}", String(rows)) }]);

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
    ws.onmessage = (event) => void handleTerminalMessage(event.data);
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
      const type = String(msg.type || "");
      if (type === "terminal.output" || type === "output") {
        appendLine("output", String(msg.data ?? msg.output ?? ""));
        return;
      }
      if (type === "terminal.closed" || type === "closed") {
        appendLine("system", String(msg.reason || copy.terminalPage.serverClosed));
        closeSocket();
        return;
      }
      if (type === "terminal.error" || type === "error") {
        appendLine("error", String(msg.error || msg.message || copy.terminalPage.terminalError));
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

  function resizeTerminal() {
    if (status !== "open") return;
    sendFrame({ type: "terminal.resize", cols, rows });
    appendLine("system", copy.terminalPage.resized.replace("{cols}", String(cols)).replace("{rows}", String(rows)));
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

  return (
    <div>
      <PageShell>
        <PageHeader
          eyebrow={copy.terminalPage.eyebrow}
          title={copy.terminalPage.title}
          detail={copy.terminalPage.detail}
          actions={<StatusBadge tone={status === "open" ? "green" : status === "error" ? "red" : "gray"}>{terminalStatusLabel(status, copy)}</StatusBadge>}
        />
        <InlineError message={error ?? serverLoadError} />

        <div className="mt-5 grid gap-6 lg:grid-cols-[360px_minmax(0,1fr)]">
          <BrutalCard>
            <form onSubmit={openTerminal} className="space-y-4">
                <Field label={copy.terminalPage.fieldAgent}>
                <select className={selectClass} value={agentId} onChange={(e) => setAgentId(e.target.value)}>
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>{server.name} ({server.status || copy.terminalPage.agentUnknownStatus})</option>
                  ))}
                </select>
              </Field>
              <div className="grid grid-cols-2 gap-3">
                <Field label={copy.terminalPage.fieldCols}>
                  <input className={inputClass} value={cols} onChange={(e) => setCols(Number(e.target.value) || 100)} />
                </Field>
                <Field label={copy.terminalPage.fieldRows}>
                  <input className={inputClass} value={rows} onChange={(e) => setRows(Number(e.target.value) || 30)} />
                </Field>
              </div>
              <div className="flex flex-wrap gap-2">
                <button disabled={opening || loading} className={buttonClass("primary")}>{copy.terminalPage.openSession}</button>
                <button type="button" onClick={resizeTerminal} className={buttonClass("secondary")}>{copy.terminalPage.resize}</button>
                <button type="button" onClick={closeSocket} className={buttonClass("danger")}>{copy.terminalPage.close}</button>
              </div>
              {sessionId ? <InlineNotice tone="pink">{copy.terminalPage.sessionLabel.replace("{id}", String(compactId(sessionId)))}</InlineNotice> : null}
            </form>
          </BrutalCard>

          <BrutalCard className="p-0">
            <div ref={outputRef} className="h-[520px] overflow-auto bg-black p-4 font-mono text-sm text-green-300">
              {lines.length === 0 ? <EmptyState title={copy.terminalPage.noOutput} /> : null}
              {lines.map((line) => (
                <div key={line.id} className={line.direction === "error" ? "text-red-300" : line.direction === "input" ? "text-pink-300" : ""}>
                  {line.text}
                </div>
              ))}
            </div>
            <div className="flex border-t-4 border-black">
              <input
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={onInputKeyDown}
                disabled={status !== "open"}
                className="min-w-0 flex-1 bg-[var(--bg-card)] px-4 py-3 font-mono text-sm font-bold outline-none"
                placeholder={status === "open" ? copy.terminalPage.inputPlaceholderOpen : copy.terminalPage.inputPlaceholderClosed}
              />
              <button type="button" onClick={sendInput} disabled={status !== "open"} className="border-l-4 border-black bg-[var(--accent-color)] px-5 font-black uppercase text-white">
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
