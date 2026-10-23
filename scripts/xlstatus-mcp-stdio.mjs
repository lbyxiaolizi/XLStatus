#!/usr/bin/env node
/**
 * XLStatus MCP Stdio-to-HTTP Bridge
 *
 * Bridges standard MCP stdio clients (Claude Desktop, Cursor, Windsurf, Cline,
 * OpenCode, Antigravity, MCP Inspector) to the XLStatus `/mcp` JSON-RPC 2.0 endpoint.
 *
 * Usage:
 *   XLSTATUS_URL="http://127.0.0.1:8080" XLSTATUS_PAT="xlp_..." node scripts/xlstatus-mcp-stdio.mjs
 *   node scripts/xlstatus-mcp-stdio.mjs --url http://127.0.0.1:8080 --token xlp_...
 */

import path from "node:path";
import { fileURLToPath } from "node:url";

export const MAX_LINE_BYTES = 1024 * 1024; // 1 MiB maximum incoming JSON-RPC line length

export function redactSensitiveTokens(input, configuredToken = "") {
  if (!input || typeof input !== "string") return "";
  let redacted = input
    .replace(/\bxlp_[A-Za-z0-9_-]+/g, "xlp_[REDACTED]")
    .replace(/\bxle_[A-Za-z0-9_-]+/g, "xle_[REDACTED]");

  const trimmedSecret = typeof configuredToken === "string" ? configuredToken.trim() : "";
  if (trimmedSecret.length >= 4 && redacted.includes(trimmedSecret)) {
    redacted = redacted.split(trimmedSecret).join("[REDACTED]");
  }
  return redacted;
}

export function parseArgs(argv, env = process.env) {
  let url = env.XLSTATUS_URL || "http://127.0.0.1:8080";
  let token = env.XLSTATUS_PAT || env.XLSTATUS_TOKEN || "";

  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i];
    if ((arg === "--url" || arg === "-u") && argv[i + 1]) {
      url = argv[i + 1];
      i += 1;
    } else if ((arg === "--token" || arg === "--pat" || arg === "-t") && argv[i + 1]) {
      token = argv[i + 1];
      i += 1;
    }
  }

  const normalizedBase = url.trim().replace(/\/+$/, "");
  let endpoint = `${normalizedBase}/mcp`;
  try {
    const parsed = new URL(endpoint);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      throw new Error(`Unsupported protocol: ${parsed.protocol}`);
    }
    endpoint = parsed.toString();
  } catch {
    endpoint = "http://127.0.0.1:8080/mcp";
  }

  return {
    endpoint,
    token: token.trim(),
  };
}

export async function forwardJsonRpc(rawLine, options = {}) {
  const endpoint = options.endpoint ?? defaultConfig.endpoint;
  const token = options.token ?? defaultConfig.token;
  const writeStdout = options.writeStdout ?? ((chunk) => process.stdout.write(chunk));
  const fetchImpl = options.fetchImpl ?? fetch;

  if (Buffer.byteLength(rawLine, "utf8") > MAX_LINE_BYTES) {
    writeStdout(
      `${JSON.stringify({
        jsonrpc: "2.0",
        id: null,
        error: {
          code: -32600,
          message: `Invalid Request: line exceeds maximum length of ${MAX_LINE_BYTES} bytes`,
        },
      })}\n`,
    );
    return;
  }

  const trimmed = rawLine.trim();
  if (!trimmed) return;

  let parsed;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    writeStdout(
      `${JSON.stringify({
        jsonrpc: "2.0",
        id: null,
        error: { code: -32700, message: "Parse error" },
      })}\n`,
    );
    return;
  }

  const isNotification =
    !Array.isArray(parsed) &&
    parsed !== null &&
    typeof parsed === "object" &&
    !("id" in parsed);

  try {
    const response = await fetchImpl(endpoint, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify(parsed),
    });

    if (isNotification) {
      return;
    }

    const text = await response.text();
    if (!response.ok) {
      const safeSnippet = redactSensitiveTokens(text.slice(0, 300), token);
      writeStdout(
        `${JSON.stringify({
          jsonrpc: "2.0",
          id: parsed?.id ?? null,
          error: {
            code: -32000,
            message: `XLStatus HTTP ${response.status}: ${safeSnippet}`,
          },
        })}\n`,
      );
      return;
    }

    const jsonResponse = JSON.parse(text);
    writeStdout(`${JSON.stringify(jsonResponse)}\n`);
  } catch (error) {
    if (isNotification) return;
    const rawMessage = error instanceof Error ? error.message : String(error);
    const safeMessage = redactSensitiveTokens(rawMessage, token);
    writeStdout(
      `${JSON.stringify({
        jsonrpc: "2.0",
        id: parsed?.id ?? null,
        error: {
          code: -32000,
          message: `XLStatus MCP bridge connection error: ${safeMessage}`,
        },
      })}\n`,
    );
  }
}

/**
 * Creates a bounded line reader that enforces MAX_LINE_BYTES on incoming chunks
 * before buffering, preventing memory exhaustion from unbounded newline-free streams.
 */
export function createBoundedLineReader(onLine, onOversizedLine, maxBytes = MAX_LINE_BYTES) {
  let buffer = "";
  let bufferBytes = 0;
  let discardingOversized = false;

  return {
    push(chunk) {
      const text = typeof chunk === "string" ? chunk : chunk.toString("utf8");
      let start = 0;

      for (let i = 0; i < text.length; i += 1) {
        if (text.charCodeAt(i) === 0x0a) {
          const segment = text.slice(start, i);
          start = i + 1;

          if (discardingOversized) {
            discardingOversized = false;
            buffer = "";
            bufferBytes = 0;
            onOversizedLine();
            continue;
          }

          const segmentBytes = Buffer.byteLength(segment, "utf8");
          if (bufferBytes + segmentBytes > maxBytes) {
            buffer = "";
            bufferBytes = 0;
            onOversizedLine();
            continue;
          }

          const fullLine = buffer ? buffer + segment : segment;
          buffer = "";
          bufferBytes = 0;
          onLine(fullLine.endsWith("\r") ? fullLine.slice(0, -1) : fullLine);
        }
      }

      if (start < text.length) {
        const remainder = text.slice(start);
        if (discardingOversized) {
          return;
        }
        const remainderBytes = Buffer.byteLength(remainder, "utf8");
        if (bufferBytes + remainderBytes > maxBytes) {
          buffer = "";
          bufferBytes = 0;
          discardingOversized = true;
          return;
        }
        buffer += remainder;
        bufferBytes += remainderBytes;
      }
    },
    end() {
      if (discardingOversized) {
        discardingOversized = false;
        buffer = "";
        bufferBytes = 0;
        onOversizedLine();
        return;
      }
      if (buffer) {
        const finalLine = buffer.endsWith("\r") ? buffer.slice(0, -1) : buffer;
        buffer = "";
        bufferBytes = 0;
        onLine(finalLine);
      }
    },
  };
}

const defaultConfig = parseArgs(process.argv);

function isExecutedDirectly() {
  const entry = process.argv[1];
  if (!entry) return false;
  try {
    return path.resolve(entry) === path.resolve(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isExecutedDirectly()) {
  if (!defaultConfig.token) {
    process.stderr.write(
      "[xlstatus-mcp-stdio] Warning: XLSTATUS_PAT is not set. Requests to /mcp require a Personal Access Token.\n",
    );
  }

  let queue = Promise.resolve();

  const reader = createBoundedLineReader(
    (line) => {
      queue = queue
        .then(() => forwardJsonRpc(line))
        .catch((err) => {
          const rawMessage = err instanceof Error ? err.message : String(err);
          const safeMessage = redactSensitiveTokens(rawMessage, defaultConfig.token);
          process.stderr.write(`[xlstatus-mcp-stdio] Internal error: ${safeMessage}\n`);
        });
    },
    () => {
      queue = queue.then(() => {
        process.stdout.write(
          `${JSON.stringify({
            jsonrpc: "2.0",
            id: null,
            error: {
              code: -32600,
              message: `Invalid Request: line exceeds maximum length of ${MAX_LINE_BYTES} bytes`,
            },
          })}\n`,
        );
      });
    },
  );

  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (chunk) => reader.push(chunk));
  process.stdin.on("end", () => reader.end());
}
