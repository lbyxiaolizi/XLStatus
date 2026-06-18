# RBAC and PAT Scopes

XLStatus uses a two-layer authorization model:

1. **Role-based** — every authenticated principal is either `admin` or `member`.
2. **Scope-based** — Personal Access Tokens (PATs) carry a list of scopes and an
   optional `server_ids` allowlist that restrict which endpoints and which
   servers the token may touch.

Cookie sessions implicitly satisfy the role layer (admins are trusted; members
can still call the PAT-self-management endpoints, etc.) and skip the scope check
on every business endpoint. PATs are the only way to authenticate without a
browser cookie, and they must present the right scope(s) for every call.

## Canonical Scopes

The set of accepted scope names is defined in
[`KNOWN_SCOPES`](../crates/server/src/auth/rbac.rs) and mirrors the table in
`plan/07-security.md`:

| Namespace     | Actions            | Notes                                       |
|---------------|--------------------|---------------------------------------------|
| `inventory`   | `read`, `delete`   | Agent enrollment and removal                |
| `server`      | `read`, `write`, `delete`, `exec` | Server inventory + remote exec |
| `service`     | `read`, `write`, `delete` | Service probes                        |
| `alert`       | `read`, `write`, `delete` | Alert rules                           |
| `task`        | `read`, `write`, `delete`, `exec` | Scheduled and on-demand tasks  |
| `ddns`        | `read`, `write`, `delete` | Dynamic DNS records                  |
| `nat`         | `read`, `write`, `delete` | NAT port mappings                  |
| `notification`| `read`, `write`, `delete` | Notification groups                |
| `transfer`    | `read`, `write`    | File transfer endpoints                      |
| `admin`       | `*`                | Catch-all for admin operations              |

Wildcards are also supported at runtime as **namespace** wildcards
(`task:*`, `nat:*`, etc.). A PAT carrying `task:*` can call any `task:*`
endpoint. The bare `*` literal is **not** allowed at creation.

## Per-Endpoint Scope Map

| Route                                                   | Method | Required scope (PAT)        |
|---------------------------------------------------------|--------|-----------------------------|
| `/api/v1/tasks`                                         | POST   | `task:write`                |
| `/api/v1/tasks`                                         | GET    | `task:read`                 |
| `/api/v1/tasks/:id`                                     | GET    | `task:read`                 |
| `/api/v1/tasks/:id`                                     | POST   | `task:write`                |
| `/api/v1/tasks/:id`                                     | DELETE | `task:delete`               |
| `/api/v1/tasks/:id/run`                                 | POST   | `task:exec`                 |
| `/api/v1/tasks/:id/runs`                                | GET    | `task:read`                 |
| `/api/v1/nat/mappings`                                  | POST   | `nat:write`                 |
| `/api/v1/nat/mappings/:id`                              | GET    | `nat:read`                  |
| `/api/v1/nat/mappings/agent/:agent_id`                  | GET    | `nat:read` + agent allowed  |
| `/api/v1/nat/mappings/all`                              | GET    | `nat:read`                  |
| `/api/v1/nat/mappings/:id`                              | POST   | `nat:write`                 |
| `/api/v1/nat/mappings/:id`                              | DELETE | `nat:delete`                |
| `/api/v1/tokens`                                        | POST   | Cookie session (admin)      |
| `/api/v1/tokens`                                        | GET    | Cookie session              |
| `/api/v1/tokens/:id`                                    | DELETE | Cookie session              |
| `/api/v1/mcp/info`                                      | GET    | PAT only (any scope)        |

A 401 is returned for missing/invalid authentication; a 403 is returned when
the scope is missing or the allowlist rejects the target server.

## Server Allowlist

A PAT may carry an optional `server_ids: [uuid, ...]` field. When set, every
call that targets a specific server must reference an id in this list:

- `POST /api/v1/tasks` — the request's `server_selector_json` is parsed and
  every UUID it contains must be in the allowlist. (The `{"all": true}` form
  is only accepted for cookie sessions or admin users.)
- `POST /api/v1/nat/mappings` — the request's `agent_id` must be in the
  allowlist.
- `GET /api/v1/nat/mappings/agent/:agent_id` — the path's agent id must be in
  the allowlist.

A PAT without `server_ids` (legacy or unscoped-server PAT) is treated as
unrestricted at the server level — the scope check still applies.

## Centralized Validators

All scope and allowlist checks live in
[`crates/server/src/auth/rbac.rs`](../crates/server/src/auth/rbac.rs):

- `KNOWN_SCOPES` — the canonical allowlist of scope names.
- `validate_pat_scopes(scopes, is_admin)` — rejects empty lists, bare `*`,
  malformed scopes, unknown scopes, and `admin:*` from non-admins.
- `validate_server_ids(ids)` — rejects non-UUID allowlist entries.
- `has_scope(session, required_scope)` — returns `true` for cookie sessions
  unconditionally; for PATs, exact match or namespace wildcard match.
- `can_access_server(session, server_id)` and `can_access_servers(session,
  ids)` — PAT allowlist filter (no-op when the PAT has no allowlist).
- `require_admin`, `require_auth`, `require_scope` — axum middleware for
  whole-route guarding.

The PAT creation/revoke handler in `crates/server/src/api/v1/pat.rs` is the
only place that mints tokens, and it calls `validate_pat_scopes` /
`validate_server_ids` directly — there are no parallel copies elsewhere.

## Tests

`crates/server/src/auth/rbac.rs` includes a `#[cfg(test)] mod tests` with 21
unit tests:

- 8 cover `validate_pat_scopes` (empty, wildcard, unknown, malformed, empty
  namespace, known, multiple, `admin:*` for non-admin).
- 4 cover `validate_server_ids` (none, empty, valid UUID, non-UUID).
- 3 cover `can_access_server*` (no allowlist, allowlist filters, batch check).
- 5 cover `has_scope` (cookie always true, exact match, namespace wildcard,
  bare wildcard, `admin:*` literal).

Together with the runtime smoke chain, these cover both unit-level semantics
and end-to-end enforcement (cookie vs. PAT, scope-missing 403,
allowlist-mismatch 403, in-allowlist 201).

## Operational Notes

- Issuing a PAT requires a **cookie session** (admins or members can self-issue
  within their own scope budget). PATs cannot mint other PATs.
- Revoking a PAT is also cookie-session only and scoped to the caller's own
  tokens.
- `admin:*` is treated as a literal scope name during `has_scope` checks. It
  does **not** namespace-wildcard into other domains; admin operations rely on
  the `require_admin` middleware or an explicit `role.is_admin()` check at the
  handler level. This is the same semantics the plan prescribes.
