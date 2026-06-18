#!/usr/bin/env bash
# M7 frontend completeness smoke verification.
# Pass criteria:
# - Next lint succeeds.
# - Dashboard resource pages and public status page exist.
# - Navigation exposes admin/member/public views.
# - Server detail UI wires file list/read/write/delete, temp transfer URLs,
#   remote config, and force update actions.
# - Terminal UI wires the WebSocket session flow.
set -euo pipefail

ROOT="${ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"

cd "$ROOT/web"
pnpm lint >/dev/null

cd "$ROOT"
python3 <<'PY'
from pathlib import Path

root = Path.cwd()

required_pages = [
    "web/app/(dashboard)/dashboard/page.tsx",
    "web/app/(dashboard)/servers/page.tsx",
    "web/app/(dashboard)/servers/[id]/page.tsx",
    "web/app/(dashboard)/services/page.tsx",
    "web/app/(dashboard)/tasks/page.tsx",
    "web/app/(dashboard)/terminal/page.tsx",
    "web/app/(dashboard)/alerts/page.tsx",
    "web/app/(dashboard)/ddns/page.tsx",
    "web/app/(dashboard)/nat/page.tsx",
    "web/app/(dashboard)/settings/page.tsx",
    "web/app/status/page.tsx",
]
for rel in required_pages:
    path = root / rel
    assert path.exists(), f"missing page: {rel}"

api = (root / "web/lib/api.ts").read_text(encoding="utf-8")
for token in [
    "listServerFiles",
    "readServerFile",
    "writeServerFile",
    "deleteServerFile",
    "getServerDownloadUrl",
    "getServerUploadUrl",
    "applyServerConfig",
    "forceUpdateServer",
    "createTerminalSession",
    "listPats",
    "createPat",
]:
    assert token in api, f"api client missing {token}"

server_detail = (root / "web/app/(dashboard)/servers/[id]/page.tsx").read_text(encoding="utf-8")
for token in [
    "Write File",
    "Download URL",
    "Upload URL",
    "Apply Config",
    "Send Update",
    "disable_command_execute",
]:
    assert token in server_detail, f"server detail UI missing {token}"

terminal = (root / "web/app/(dashboard)/terminal/page.tsx").read_text(encoding="utf-8")
for token in [
    "createTerminalSession",
    "new WebSocket",
    "terminal.input",
    "terminal.resize",
]:
    assert token in terminal, f"terminal UI missing {token}"

status = (root / "web/app/status/page.tsx").read_text(encoding="utf-8")
for token in [
    "listServers(100, 0, true)",
    "listServices(100, 0, true)",
    "No public data",
]:
    assert token in status, f"public status view missing {token}"

nav = (root / "web/app/components/Navigation.tsx").read_text(encoding="utf-8")
for token in [
    "adminOnly",
    "publicNavigation",
    "md:hidden",
    "isAdmin(user)",
]:
    assert token in nav, f"navigation/permission view missing {token}"

print("M7 UI PASS (dashboard pages, public view, permissions, terminal, file transfer/config/update UI)")
PY
