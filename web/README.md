# XLStatus Web Frontend (`web/`)

The **XLStatus Web Ops Workbench** is a high-density, neo-brutalist infrastructure monitoring and AI cluster operations frontend built with **Next.js 16 (App Router)**, **React 19**, **Tailwind CSS v4**, and **Full Strict TypeScript**.

---

## Tech Stack & Strict TypeScript Guarantees

- **Framework**: Next.js `16.2.9` (App Router) + React `19.2.4`
- **Styling & Theme**: Tailwind CSS v4 (`@tailwindcss/postcss`) with dual Neo-Brutalist palettes:
  - **Light Mode**: High-contrast ink-black borders (`#121212`) and vibrant pink accents on `#f8f8f8`
  - **Dark Mode**: Crisp emerald telemetry accents on `#121212`
- **Visualizations**: `d3-geo` interactive global node projection (`WorldServerMap`) and real-time SVG sparklines
- **Zero-Compromise TypeScript (`tsconfig.json`)**:
  - Enforces `"strict": true`, `"noImplicitAny": true`, `"strictNullChecks": true`
  - Enforces `"noUncheckedIndexedAccess": true` (all array/record indexing is explicitly bounds-checked)
  - Enforces `"exactOptionalPropertyTypes": true` and `"noPropertyAccessFromIndexSignature": true`
  - Enforces `"noUnusedLocals": true`, `"noUnusedParameters": true`, and `"noImplicitReturns": true`

---

## Directory Structure

```text
web/
├── app/
│   ├── (dashboard)/             Authenticated Ops Workbench route group
│   │   ├── page.tsx             Fleet Command Center (throughput cards, matrix, world map)
│   │   ├── servers/             Server inventory, batch operations, files, config & WebSocket PTY terminal
│   │   ├── services/            HTTP/TCP/ICMP service probes, SLA history bars & TLS expiry
│   │   ├── alerts/              Multi-condition alert rules, active incidents & notification channels
│   │   ├── tasks/               Batch command execution & Cron scheduler with SRE playbooks
│   │   ├── network/             Multi-provider DDNS & NAT reverse tunnel management
│   │   ├── mcp/                 AI Cluster Copilot, MCP 2024-11-05 Playground & IDE Connect Wizard
│   │   ├── tokens/              Scoped Personal Access Tokens (xlp_) & Agent Enrollment Tokens (xle_)
│   │   ├── users/               RBAC user management & credential revocation
│   │   └── settings/            Segmented Settings Center (Branding, Theme, 2FA TOTP, Agent Install, Backup)
│   ├── status/                  Public Status Page (/status) with SSR metadata & Cards/Table/Map views
│   ├── login/                   Argon2id + TOTP 2FA & OIDC login flow
│   ├── components/              Shared UI architecture
│   │   ├── Navigation.tsx       PC Priority+ ResizeObserver adaptive navbar + ⌘K Command Palette
│   │   ├── ServerCard.tsx       Telemetry card with inline selection, sparklines & quick terminal launch
│   │   ├── WorldServerMap.tsx   d3-geo global fleet distribution map
│   │   └── ...                  Modals, status badges, and layout primitives
│   └── lib/                     Frontend domain helpers & unit-tested security utilities
│       ├── security.ts          URL/CSS/Path/Shell sanitizers (encodePathSegment, resolveSafeApiUrl, etc.)
│       ├── security.test.mts    Node.js native unit test suite for frontend security boundaries
│       └── ...                  Formatting and telemetry calculation utilities
├── lib/
│   ├── api.ts                   Typed REST, WebSocket, and MCP Studio client with CSRF & TOTP headers
│   └── i18n.ts                  Simplified Chinese (zh-CN) & English locale dictionary and helpers
├── next.config.ts               Global security response headers (nosniff, DENY, Permissions-Policy)
├── tsconfig.json                Full strict-mode TypeScript configuration
└── package.json                 Scripts and pinned pnpm@10.23.0 package manager configuration
```

---

## Key UI & UX Architecture

### 1. PC Priority+ Adaptive Top Navigation (`app/components/Navigation.tsx`)
Instead of wrapping nav items into multiple rows or hiding everything behind a mobile drawer on medium viewports, `Navigation.tsx` uses a hidden off-screen measurement container paired with a `ResizeObserver`:
- Continuously calculates the exact pixel width available between the brand header and right-hand utility controls.
- Renders as many primary navigation links (with icons) as fit on a single row.
- Automatically collapses any overflowing routes into a **`More (+N) ▾`** dropdown badge with active-route awareness.

### 2. Global `⌘K` / `Ctrl+K` Command Palette (`app/components/Navigation.tsx`)
Pressing `⌘K` (macOS) or `Ctrl+K` (Windows/Linux) from any screen opens an instant keyboard-navigable command palette supporting:
- Direct jump to any workbench route (`/servers`, `/services`, `/alerts`, `/tasks`, `/network`, `/mcp`, `/tokens`, `/settings`).
- Live fuzzy search across online/offline fleet servers with one-key navigation to node details or WebSocket PTY terminal.
- Quick execution of theme toggles (Light / Dark) and workspace actions.

### 3. AI Cluster Copilot & MCP `2024-11-05` Workbench (`app/(dashboard)/mcp/page.tsx`)
A dedicated three-tab AI operations studio:
1. **Cluster Health & Diagnostics**: Runs `cluster.diagnose` to compute a `0–100` health score, surface root-cause anomalies, execute 6 cross-platform read-only SRE inspection playbooks (`server.inspect`), and generate live-context SRE prompts.
2. **MCP Protocol Playground**: Interactive debugger for all **19 Tools**, **6 Live Resources**, and **4 Prompts** in either Cookie Session Studio mode (`/api/v1/mcp/studio`) or raw `POST /mcp` JSON-RPC 2.0 mode with `xlp_*` Bearer tokens.
3. **One-Click Agent / IDE Connect Wizard**: Issues least-privilege PATs (*Read-Only Audit*, *Standard Troubleshooting*, *Full Autonomous Ops*) with optional `server_ids` allowlists and generates ready-to-paste configs for Claude Code, Cursor, Windsurf, Antigravity, and Claude Desktop (`stdio`).

---

## Frontend Security Architecture (`app/lib/security.ts` & `lib/api.ts`)

The frontend enforces strict client-side defense-in-depth alongside the Rust server:
- **WHATWG Dot-Segment Path Traversal Prevention**: Every dynamic route parameter in `lib/api.ts` passes through `encodePathSegment()`, rejecting `.`, `..`, `%2e`, and `%2e%2e` so `fetch()` URL normalization cannot traverse to parent API endpoints.
- **Same-Origin Temporary Transfer Verification**: `resolveSafeApiUrl()` validates that single-use file download/upload URLs (`xlt_`) point to the configured `NEXT_PUBLIC_API_URL` origin before initiating transfers.
- **XSS & CSS Injection Sanitizers**: `sanitizeExternalUrl()`, `sanitizeImageSrcUrl()`, and `sanitizeCssColorValue()` block `javascript:`, `vbscript:`, `data:text/html`, and CSS `url(...)` / semicolon breakout sequences in custom branding and public status themes.
- **Shell Snippet Escaping**: `escapeShellDoubleQuoted()` safely escapes hostnames and group tags interpolated into generated `install-agent.sh` bootstrap commands.
- **Session-Scoped Token Storage**: Newly minted enrollment token secrets in `/tokens` are held only in `sessionStorage` (never persisted in `localStorage`) and are wiped upon `apiClient.logout()`.

---

## Development & Verification Commands

### Prerequisites
- **Node.js**: `20+`
- **Package Manager**: Corepack enabled with pinned `pnpm@10.23.0`
- **Backend Server**: `xlstatus-server` running (default `http://localhost:8080`) with `CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000`

### Install Dependencies

```bash
corepack enable
pnpm install --frozen-lockfile
```

### Start Development Server

```bash
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

- Workbench UI: `http://localhost:3000`
- Public Status Page: `http://localhost:3000/status`
- AI & MCP Studio: `http://localhost:3000/mcp`

### Lint, Strict Typecheck, Unit Tests & Production Build

```bash
# Run ESLint (0 warnings / 0 errors gate)
pnpm lint

# Run full strict-mode TypeScript compiler check
pnpm typecheck

# Run frontend security & utility unit tests (Node.js native test runner)
pnpm test:unit

# Build and start production bundle
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```
