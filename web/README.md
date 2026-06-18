# XLStatus Web

This is the Next.js dashboard for XLStatus.
It uses the BOLD. neo-brutalist palette: light mode is pink/black on `#f8f8f8`, dark mode is emerald on `#121212`, and the selected mode is stored in `localStorage.darkMode`.

## Requirements

- Node.js 20+
- Corepack enabled, using the pinned `pnpm@10.23.0` from `package.json`
- XLStatus server reachable through `NEXT_PUBLIC_API_URL`

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
