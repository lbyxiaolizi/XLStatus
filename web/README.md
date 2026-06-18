# XLStatus Web

This is the Next.js dashboard for XLStatus.
It uses the BOLD. neo-brutalist palette: light mode is pink/black on `#f8f8f8`, dark mode is emerald on `#121212`, and the selected mode is stored in `localStorage.darkMode`.
The current UI locale is Simplified Chinese (`zh-CN`). Locale defaults and shared messages live in [`lib/i18n.ts`](./lib/i18n.ts).

## Requirements

- Node.js 20+
- Corepack enabled, using the pinned `pnpm@10.23.0` from `package.json`
- XLStatus server reachable through `NEXT_PUBLIC_API_URL`
- The API server must allow this Web UI origin through `CORS_ALLOWED_ORIGINS` or `server.cors_allowed_origins`

## Install

```bash
corepack enable
pnpm install --frozen-lockfile
```

## Development

Start the Rust server first, then run:

```bash
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm dev
```

The API server must allow the Web UI origin through `CORS_ALLOWED_ORIGINS` or `server.cors_allowed_origins`. The default local origin is `http://localhost:3000`; if Next.js uses another port, add that exact origin before starting the server:

```bash
CORS_ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000
```

Open `http://localhost:3000`.
Open `http://localhost:3000/status` before login to verify the public status endpoint.

## Production Build

```bash
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```

## Backend Contract

- Public status: `GET /api/v1/public/status`, no session required.
- Authenticated dashboard flows use cookie sessions and CSRF headers through `web/lib/api.ts`.
- Server files/config/update, service CRUD/probe, task CRUD/run, terminal WebSocket, alert rules/events, DDNS, NAT, and PAT settings are wired to the current `/api/v1/*` server routes.

## i18n

- Default locale: `zh-CN`
- Supported locales: `zh-CN`
- App Router locale configuration is exported from `lib/i18n.ts`; `next.config.ts` does not use the legacy Pages Router `i18n` field
- User-visible shared strings should be added to `lib/i18n.ts`; protocol values and backend enum values should stay unchanged
