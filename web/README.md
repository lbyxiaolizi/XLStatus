# XLStatus Web

This is the Next.js dashboard for XLStatus.

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

## Production Build

```bash
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm build
NEXT_PUBLIC_API_URL=http://localhost:8080 pnpm start
```
