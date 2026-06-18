"use client";

import { FormEvent, KeyboardEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import Navigation from "@/app/components/Navigation";
import {
  EmptyState,
  Field,
  InlineError,
  InlineNotice,
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

const defaultCols = 100;
const defaultRows = 30;

export default function TerminalPage() {
  const [servers, setServers] = useState<Server[]>([]);
  const [agentId, setAgentId] = useState("");
  const [cols, setCols] = useState(defaultCols);
  const [rows, setRows] = useState(defaultRows);
  const [input, setInput] = useState("");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [status, setStatus] = useState<TerminalStatus>("idle");
  const [loading, setLoading] = useState(true);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lines, setLines] = useState<TerminalLine[]>([
    {
      id: 1,
      direction: "system",
      text: "Select an agent and open a terminal session.",
    },
  ]);
  const wsRef = useRef<WebSocket | null>(null);
  const lineIdRef = useRef(2);
  const outputRef = useRef<HTMLDivElement | null>(null);

  const selectedAgent = useMemo(
    () => servers.find((server) => server.id === agentId),
    [agentId, servers],
  );

  const appendLine = useCallback((direction: TerminalLine["direction"], text: string) => {
    setLines((prev) =>
      [
        ...prev,
        {
          id: lineIdRef.current++,
          direction,
          text,
        },
      ].slice(-400),
    );
  }, []);

  const closeSocket = useCallback(() => {
    wsRef.current?.close();
    wsRef.current = null;
  }, []);

  const loadServers = useCallback(async () => {
    setError(null);
    try {
      setLoading(true);
      const response = await apiClient.listServers(200, 0);
      if (response.success && response.data) {
        const loaded = ((response.data.servers as Server[]) ?? []).filter((server) => server.id);
        setServers(loaded);
        setAgentId((current) => current || loaded[0]?.id || "");
      } else {
        setError(responseError(response));
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      void loadServers();
    }, 0);
    return () => window.clearTimeout(timeoutId);
  }, [loadServers]);

  useEffect(() => {
    outputRef.current?.scrollTo({
      top: outputRef.current.scrollHeight,
      behavior: "smooth",
    });
  }, [lines]);

  useEffect(() => {
    return () => closeSocket();
  }, [closeSocket]);

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
    setLines([
      {
        id: lineIdRef.current++,
        direction: "system",
        text: `Opening ${selectedAgent?.name || compactId(agentId)} (${cols}x${rows})...`,
      },
    ]);

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
      appendLine("error", "Terminal session response did not include a session id.");
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

    ws.onmessage = (event) => {
      void handleTerminalMessage(event.data);
    };

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
    if (!value || status !== "open") {
      return;
    }

    sendFrame({ type: "terminal.input", data: `${value}\n` });
    appendLine("input", value);
    setInput("");
  }

  function sendResize() {
    if (status !== "open") {
      return;
    }
    sendFrame({ type: "terminal.resize", cols, rows });
    appendLine("system", `Resize sent: ${cols}x${rows}.`);
  }

  function sendFrame(payload: Record<string, unknown>) {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(payload));
    }
  }

  function handleInputKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      event.preventDefault();
      sendInput();
    }
  }

  const statusTone =
    status === "open" ? "green" : status === "connecting" ? "yellow" : status === "error" ? "red" : "gray";

  return (
    <div className="min-h-screen bg-gray-100">
      <Navigation />
      <PageShell>
        <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div>
            <h1 className="text-2xl font-bold text-gray-900">Terminal</h1>
            <p className="mt-1 text-sm text-gray-500">Interactive agent sessions over WebSocket.</p>
          </div>
          <StatusBadge tone={statusTone}>{status}</StatusBadge>
        </div>

        <div className="grid gap-6 lg:grid-cols-[320px_minmax(0,1fr)]">
          <form onSubmit={openTerminal} className="rounded-lg bg-white p-5 shadow">
            <div className="grid gap-4">
              {error ? <InlineError message={error} /> : null}
              <Field label="Agent">
                <select
                  value={agentId}
                  onChange={(event) => setAgentId(event.target.value)}
                  className={selectClass}
                  disabled={loading || opening || status === "open"}
                >
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>
                      {server.name || compactId(server.id)} {server.status ? `(${server.status})` : ""}
                    </option>
                  ))}
                </select>
              </Field>
              <div className="grid grid-cols-2 gap-3">
                <Field label="Cols">
                  <input
                    type="number"
                    min={20}
                    max={240}
                    value={cols}
                    onChange={(event) => setCols(clampSize(event.target.value, 20, 240))}
                    className={inputClass}
                  />
                </Field>
                <Field label="Rows">
                  <input
                    type="number"
                    min={8}
                    max={80}
                    value={rows}
                    onChange={(event) => setRows(clampSize(event.target.value, 8, 80))}
                    className={inputClass}
                  />
                </Field>
              </div>
              <div className="flex flex-wrap gap-2">
                <button
                  type="submit"
                  className={buttonClass("primary")}
                  disabled={loading || opening || !agentId || status === "open"}
                >
                  {opening ? "Opening..." : "Open Terminal"}
                </button>
                <button
                  type="button"
                  onClick={sendResize}
                  className={buttonClass("secondary")}
                  disabled={status !== "open"}
                >
                  Send Resize
                </button>
                <button
                  type="button"
                  onClick={closeSocket}
                  className={buttonClass("secondary")}
                  disabled={status !== "open" && status !== "connecting"}
                >
                  Close
                </button>
              </div>
              {sessionId ? (
                <div className="rounded border border-gray-200 bg-gray-50 px-3 py-2 text-xs text-gray-500">
                  Session {compactId(sessionId)}
                </div>
              ) : null}
              {!loading && servers.length === 0 ? (
                <InlineNotice tone="yellow">No accessible agents were returned.</InlineNotice>
              ) : null}
            </div>
          </form>

          <div className="rounded-lg bg-white shadow">
            <div
              ref={outputRef}
              className="h-[520px] overflow-auto rounded-t-lg bg-gray-950 p-4 font-mono text-sm leading-6 text-gray-100"
              aria-label="Terminal output"
            >
              {lines.length === 0 ? (
                <EmptyState title="No terminal output" />
              ) : (
                lines.map((line) => (
                  <div key={line.id} className={lineClass(line.direction)}>
                    {line.direction === "input" ? "$ " : ""}
                    {line.text}
                  </div>
                ))
              )}
            </div>
            <div className="grid gap-3 border-t border-gray-200 p-4 md:grid-cols-[minmax(0,1fr)_auto]">
              <input
                value={input}
                onChange={(event) => setInput(event.target.value)}
                onKeyDown={handleInputKeyDown}
                className={inputClass}
                placeholder={status === "open" ? "Type a command and press Enter" : "Open a terminal first"}
                disabled={status !== "open"}
              />
              <button
                type="button"
                onClick={sendInput}
                className={buttonClass("primary")}
                disabled={status !== "open" || !input}
              >
                Send
              </button>
            </div>
          </div>
        </div>
      </PageShell>
    </div>
  );
}

function buildTerminalWsUrl(sessionId: string): string {
  const apiBase = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8080";
  const u = new URL(apiBase);
  const protocol = u.protocol === "https:" ? "wss" : "ws";
  return `${protocol}://${u.host}/ws/terminal/${encodeURIComponent(sessionId)}`;
}

function clampSize(value: string, min: number, max: number): number {
  const parsed = Number(value);
  if (Number.isNaN(parsed)) {
    return min;
  }
  return Math.min(Math.max(Math.round(parsed), min), max);
}

function lineClass(direction: TerminalLine["direction"]): string {
  if (direction === "input") {
    return "whitespace-pre-wrap break-words text-blue-200";
  }
  if (direction === "error") {
    return "whitespace-pre-wrap break-words text-red-300";
  }
  if (direction === "system") {
    return "whitespace-pre-wrap break-words text-gray-400";
  }
  return "whitespace-pre-wrap break-words text-gray-100";
}
