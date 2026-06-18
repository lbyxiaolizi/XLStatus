"use client";

import { FormEvent, KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import Navigation from "@/app/components/Navigation";
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
import { apiClient } from "@/lib/api";

interface Server {
  id: string;
  name: string;
  status?: string;
}

type TerminalStatus = "idle" | "connecting" | "open" | "closed" | "error";

interface TerminalLine {
  id: number;
  direction: "system" | "input" | "output" | "error";
  text: string;
}

export default function TerminalPage() {
  const [servers, setServers] = useState<Server[]>([]);
  const [agentId, setAgentId] = useState("");
  const [cols, setCols] = useState(100);
  const [rows, setRows] = useState(30);
  const [input, setInput] = useState("");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [status, setStatus] = useState<TerminalStatus>("idle");
  const [loading, setLoading] = useState(true);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lines, setLines] = useState<TerminalLine[]>([
    { id: 1, direction: "system", text: "Select an agent and open a terminal session." },
  ]);
  const wsRef = useRef<WebSocket | null>(null);
  const lineIdRef = useRef(2);
  const outputRef = useRef<HTMLDivElement | null>(null);

  const selectedAgent = useMemo(() => servers.find((server) => server.id === agentId), [agentId, servers]);

  const appendLine = useCallback((direction: TerminalLine["direction"], text: string) => {
    setLines((prev) => [...prev, { id: lineIdRef.current++, direction, text }].slice(-400));
  }, []);

  const closeSocket = useCallback(() => {
    wsRef.current?.close();
    wsRef.current = null;
  }, []);

  const loadServers = useCallback(async () => {
    setLoading(true);
    setError(null);
    const response = await apiClient.listServers(200, 0);
    setLoading(false);
    if (response.success && response.data) {
      const loaded = ((response.data.servers as Server[]) ?? []).filter((server) => server.id);
      setServers(loaded);
      setAgentId((current) => current || loaded[0]?.id || "");
    } else {
      setError(responseError(response));
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadServers();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadServers]);

  useEffect(() => {
    outputRef.current?.scrollTo({ top: outputRef.current.scrollHeight, behavior: "smooth" });
  }, [lines]);

  useEffect(() => closeSocket, [closeSocket]);

  async function openTerminal(event?: FormEvent) {
    event?.preventDefault();
    if (!agentId) {
      setError("Select an agent before opening a terminal.");
      return;
    }

    closeSocket();
    setError(null);
    setOpening(true);
    setStatus("connecting");
    setLines([{ id: lineIdRef.current++, direction: "system", text: `Opening ${selectedAgent?.name || compactId(agentId)} (${cols}x${rows})...` }]);

    const response = await apiClient.createTerminalSession(agentId, cols, rows);
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
      setError("Terminal session response did not include a session id.");
      return;
    }

    setSessionId(nextSessionId);
    connectTerminal(nextSessionId);
  }

  function connectTerminal(nextSessionId: string) {
    const ws = new WebSocket(buildTerminalWsUrl(nextSessionId));
    wsRef.current = ws;

    ws.onopen = () => {
      setOpening(false);
      setStatus("open");
      appendLine("system", `Connected to session ${compactId(nextSessionId)}.`);
      sendFrame({ type: "terminal.resize", cols, rows });
    };
    ws.onmessage = (event) => void handleTerminalMessage(event.data);
    ws.onerror = () => {
      setOpening(false);
      setStatus("error");
      appendLine("error", "WebSocket error.");
    };
    ws.onclose = () => {
      setOpening(false);
      setStatus((current) => (current === "error" ? "error" : "closed"));
      appendLine("system", "Terminal closed.");
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
        appendLine("system", String(msg.reason || "Terminal closed by server."));
        closeSocket();
        return;
      }
      if (type === "terminal.error" || type === "error") {
        appendLine("error", String(msg.error || msg.message || "Terminal error."));
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
    appendLine("system", `Resized to ${cols}x${rows}.`);
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
    <div className="min-h-screen">
      <Navigation />
      <PageShell>
        <PageHeader
          eyebrow="Remote Shell"
          title="Terminal"
          detail="通过后端 WebSocket 建立 agent 终端会话。"
          actions={<StatusBadge tone={status === "open" ? "green" : status === "error" ? "red" : "gray"}>{status}</StatusBadge>}
        />
        <InlineError message={error} />

        <div className="mt-5 grid gap-6 lg:grid-cols-[360px_minmax(0,1fr)]">
          <BrutalCard>
            <form onSubmit={openTerminal} className="space-y-4">
              <Field label="Agent">
                <select className={selectClass} value={agentId} onChange={(e) => setAgentId(e.target.value)}>
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>{server.name} ({server.status || "unknown"})</option>
                  ))}
                </select>
              </Field>
              <div className="grid grid-cols-2 gap-3">
                <Field label="Cols">
                  <input className={inputClass} value={cols} onChange={(e) => setCols(Number(e.target.value) || 100)} />
                </Field>
                <Field label="Rows">
                  <input className={inputClass} value={rows} onChange={(e) => setRows(Number(e.target.value) || 30)} />
                </Field>
              </div>
              <div className="flex flex-wrap gap-2">
                <button disabled={opening || loading} className={buttonClass("primary")}>Open Session</button>
                <button type="button" onClick={resizeTerminal} className={buttonClass("secondary")}>Resize</button>
                <button type="button" onClick={closeSocket} className={buttonClass("danger")}>Close</button>
              </div>
              {sessionId ? <InlineNotice tone="pink">Session {compactId(sessionId)}</InlineNotice> : null}
            </form>
          </BrutalCard>

          <BrutalCard className="p-0">
            <div ref={outputRef} className="h-[520px] overflow-auto bg-black p-4 font-mono text-sm text-green-300">
              {lines.length === 0 ? <EmptyState title="No terminal output" /> : null}
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
                placeholder={status === "open" ? "type command and press Enter" : "open terminal first"}
              />
              <button type="button" onClick={sendInput} disabled={status !== "open"} className="border-l-4 border-black bg-[var(--accent-color)] px-5 font-black uppercase text-white">
                Send
              </button>
            </div>
          </BrutalCard>
        </div>
      </PageShell>
    </div>
  );
}

function buildTerminalWsUrl(sessionId: string): string {
  const apiBase = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8080";
  const url = new URL(apiBase);
  return `${url.protocol === "https:" ? "wss" : "ws"}://${url.host}/ws/terminal/${encodeURIComponent(sessionId)}`;
}
