#!/usr/bin/env python3
"""Import an XLStatus SQLite database into an initialized PostgreSQL schema.

The target PostgreSQL database must already contain the XLStatus tables. Start
the same XLStatus server version against the target PostgreSQL database once to
run migrations before using this script.
"""

from __future__ import annotations

import argparse
import re
import sqlite3
import subprocess
import sys
from typing import TextIO


COPY_BATCH_SIZE = 1_000
IDENTIFIER_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]{0,62}$")
_URL_CREDENTIALS_RE = re.compile(
    r"\b([A-Za-z][A-Za-z0-9+.-]*://)([^/\s:@]+(?::[^/\s@]*)?@)"
)
_URL_SECRET_PARAM_RE = re.compile(
    r"([?&](?:password|sslpassword|passfile|token|secret)=)([^&\s'\"]+)",
    re.IGNORECASE,
)
_KEYVAL_SECRET_RE = re.compile(
    r"\b((?:PGPASSWORD|password|sslpassword|passfile)\s*=\s*)(?:'[^']*'|\"[^\"]*\"|\S+)",
    re.IGNORECASE,
)

# Keep this list in sync with crates/server/migrations. The import command may
# run against a PostgreSQL database whose public schema also contains unrelated
# application or extension tables; those tables must never be locked, checked,
# or truncated by an XLStatus migration.
XLSTATUS_TABLES = frozenset(
    {
        "agent_ip_events",
        "agents",
        "alert_events",
        "alert_rules",
        "audit_logs",
        "ddns_configs",
        "ddns_history",
        "enrollment_tokens",
        "nat_mappings",
        "nat_tunnels",
        "nat_usage_windows",
        "notification_group_members",
        "notification_groups",
        "notifications",
        "oauth_accounts",
        "personal_access_tokens",
        "server_group_members",
        "server_groups",
        "server_owner_transfers",
        "servers",
        "service_history",
        "service_results",
        "service_servers",
        "services",
        "sessions",
        "system_settings",
        "task_runs",
        "tasks",
        "temporary_transfer_tokens",
        "transfers",
        "users",
        "waf_bans",
        "waf_events",
        "xlstatus_metric_retention_policies",
    }
)


def redact_sensitive_text(text: str) -> str:
    redacted = _URL_CREDENTIALS_RE.sub(r"\1[REDACTED]@", text)
    redacted = _URL_SECRET_PARAM_RE.sub(r"\1[REDACTED]", redacted)
    redacted = _KEYVAL_SECRET_RE.sub(r"\1[REDACTED]", redacted)
    return redacted


def validate_identifier(name: str, kind: str = "identifier") -> str:
    if not isinstance(name, str) or not IDENTIFIER_RE.fullmatch(name):
        raise ValueError(f"invalid SQL {kind}: {name!r}")
    return name


def validate_table_name(
    table: str, allowed_tables: frozenset[str] = XLSTATUS_TABLES
) -> str:
    validate_identifier(table, "table name")
    if table not in allowed_tables:
        raise ValueError(f"table is not in XLStatus allowlist: {table!r}")
    return table


def run(cmd: list[str], input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            cmd,
            input=input_text,
            text=True,
            capture_output=True,
            check=True,
        )
    except subprocess.CalledProcessError as exc:
        redacted_stdout = redact_sensitive_text(exc.stdout) if exc.stdout else exc.stdout
        redacted_stderr = redact_sensitive_text(exc.stderr) if exc.stderr else exc.stderr
        if redacted_stdout:
            sys.stderr.write(redacted_stdout)
        if redacted_stderr:
            sys.stderr.write(redacted_stderr)
        redacted_cmd = [redact_sensitive_text(part) for part in exc.cmd]
        raise subprocess.CalledProcessError(
            exc.returncode,
            redacted_cmd,
            output=redacted_stdout,
            stderr=redacted_stderr,
        ) from None


def psql(
    container: str,
    user: str,
    database: str,
    sql: str,
    input_text: str | None = None,
    tuples: bool = False,
) -> str:
    cmd = [
        "docker",
        "exec",
        "-i",
        container,
        "psql",
        "-U",
        user,
        "-d",
        database,
        "-v",
        "ON_ERROR_STOP=1",
        "-q",
    ]
    if tuples:
        cmd.extend(["-At", "-F", "\t"])
    cmd.extend(["-c", sql])
    return run(cmd, input_text).stdout


def psql_command(container: str, user: str, database: str) -> list[str]:
    return [
        "docker",
        "exec",
        "-i",
        container,
        "psql",
        "-U",
        user,
        "-d",
        database,
        "-v",
        "ON_ERROR_STOP=1",
        "-q",
    ]


def qident(name: str) -> str:
    validate_identifier(name)
    return '"' + name.replace('"', '""') + '"'


def qliteral(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def load_pg_columns(args: argparse.Namespace) -> dict[str, list[dict[str, str]]]:
    column_sql = """
SELECT table_name, column_name, data_type, udt_name, is_nullable, COALESCE(column_default, '')
FROM information_schema.columns AS columns
WHERE columns.table_schema = 'public'
  AND EXISTS (
      SELECT 1
      FROM information_schema.tables AS tables
      WHERE tables.table_schema = columns.table_schema
        AND tables.table_name = columns.table_name
        AND tables.table_type = 'BASE TABLE'
  )
ORDER BY table_name, ordinal_position
"""
    pg_columns: dict[str, list[dict[str, str]]] = {}
    for line in psql(args.container, args.user, args.database, column_sql, tuples=True).splitlines():
        table, column, data_type, udt_name, nullable, default = line.split("\t", 5)
        if table not in XLSTATUS_TABLES:
            continue
        validate_table_name(table)
        validate_identifier(column, "column name")
        pg_columns.setdefault(table, []).append(
            {
                "name": column,
                "data_type": data_type,
                "udt_name": udt_name,
                "nullable": nullable,
                "default": default,
            }
        )
    if not pg_columns:
        raise RuntimeError(
            "no PostgreSQL tables found; start XLStatus against the target "
            "PostgreSQL database once so migrations create the schema"
        )
    return pg_columns


def ordered_tables(args: argparse.Namespace, pg_tables: list[str]) -> list[str]:
    fk_sql = """
SELECT tc.table_name AS child_table, ccu.table_name AS parent_table
FROM information_schema.table_constraints AS tc
JOIN information_schema.key_column_usage AS kcu
  ON tc.constraint_name = kcu.constraint_name
 AND tc.table_schema = kcu.table_schema
JOIN information_schema.constraint_column_usage AS ccu
  ON ccu.constraint_name = tc.constraint_name
 AND ccu.table_schema = tc.table_schema
WHERE tc.table_schema = 'public'
  AND tc.constraint_type = 'FOREIGN KEY'
ORDER BY child_table, parent_table
"""
    dependencies = {validate_table_name(table): set() for table in pg_tables}
    for line in psql(args.container, args.user, args.database, fk_sql, tuples=True).splitlines():
        child, parent = line.split("\t", 1)
        if child in dependencies and parent in dependencies and child != parent:
            dependencies[child].add(parent)

    ordered: list[str] = []
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(table: str) -> None:
        if table in visited:
            return
        if table in visiting:
            return
        visiting.add(table)
        for parent in sorted(dependencies.get(table, ())):
            visit(parent)
        visiting.remove(table)
        visited.add(table)
        ordered.append(table)

    for table in sorted(pg_tables):
        visit(table)
    return ordered


def sqlite_metadata(sqlite_path: str) -> tuple[sqlite3.Connection, set[str], dict[str, list[str]]]:
    escaped_path = (
        sqlite_path.replace("%", "%25").replace("?", "%3f").replace("#", "%23")
    )
    try:
        sqlite = sqlite3.connect(f"file:{escaped_path}?mode=ro", uri=True)
    except sqlite3.Error as exc:
        raise RuntimeError(
            f"failed to open SQLite backup: {redact_sensitive_text(str(exc))}"
        ) from None
    sqlite.row_factory = sqlite3.Row
    sqlite_tables = {
        validate_table_name(row[0])
        for row in sqlite.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
        )
        if row[0] in XLSTATUS_TABLES
    }
    sqlite_columns: dict[str, list[str]] = {}
    for table in sqlite_tables:
        sqlite_columns[table] = [
            validate_identifier(row[1], "column name")
            for row in sqlite.execute(f"PRAGMA table_info({qident(table)})")
        ]
    return sqlite, sqlite_tables, sqlite_columns


def convert(value: object, meta: dict[str, str]) -> str | None:
    if value is None:
        return None
    if isinstance(value, bytes):
        value = value.decode("utf-8")
    if meta["data_type"] == "boolean" or meta["udt_name"] == "bool":
        if isinstance(value, int):
            return "true" if value != 0 else "false"
        lowered = str(value).strip().lower()
        if lowered in {"1", "t", "true", "y", "yes", "on"}:
            return "true"
        if lowered in {"0", "f", "false", "n", "no", "off"}:
            return "false"
    if (
        str(value) == ""
        and meta["nullable"] == "YES"
        and meta["udt_name"] in {"uuid", "timestamptz", "timestamp"}
    ):
        return None
    return str(value)


COPY_TEXT_ESCAPES = {
    "\\": "\\\\",
    "\b": "\\b",
    "\f": "\\f",
    "\n": "\\n",
    "\r": "\\r",
    "\t": "\\t",
    "\v": "\\v",
}


def encode_copy_text(value: str | None) -> str:
    if value is None:
        return "\\N"
    return "".join(COPY_TEXT_ESCAPES.get(char, char) for char in value)


def write_target_setup(stream: TextIO, pg_tables: list[str], truncate: bool) -> None:
    quoted_tables = ", ".join(qident(table) for table in pg_tables)
    stream.write("BEGIN;\n")
    stream.write(f"LOCK TABLE {quoted_tables} IN ACCESS EXCLUSIVE MODE;\n")
    if truncate:
        # Omitting CASCADE makes an unknown table that references XLStatus data
        # fail and roll back the import instead of truncating that table too.
        stream.write(f"TRUNCATE {quoted_tables} RESTART IDENTITY;\n")
        return

    for table in pg_tables:
        stream.write(
            "DO $xlstatus_empty$\n"
            "BEGIN\n"
            f"  IF EXISTS (SELECT 1 FROM {qident(table)} LIMIT 1) THEN\n"
            "    RAISE EXCEPTION 'target PostgreSQL table is not empty: %', "
            f"{qliteral(table)}\n"
            "      USING HINT = 'rerun with --truncate only after verifying a backup';\n"
            "  END IF;\n"
            "END\n"
            "$xlstatus_empty$;\n"
        )


def stream_table(
    stream: TextIO,
    sqlite: sqlite3.Connection,
    table: str,
    columns: list[dict[str, str]],
) -> int:
    select_sql = (
        "SELECT "
        + ", ".join(qident(meta["name"]) for meta in columns)
        + " FROM "
        + qident(table)
    )
    cursor = sqlite.execute(select_sql)
    batch = cursor.fetchmany(COPY_BATCH_SIZE)
    if not batch:
        return 0

    stream.write(
        "\\copy "
        + qident(table)
        + " ("
        + ", ".join(qident(meta["name"]) for meta in columns)
        + ") FROM STDIN WITH (FORMAT text, NULL '\\N')\n"
    )
    row_count = 0
    while batch:
        for row in batch:
            converted = [convert(row[meta["name"]], meta) for meta in columns]
            stream.write("\t".join(encode_copy_text(value) for value in converted) + "\n")
            row_count += 1
        batch = cursor.fetchmany(COPY_BATCH_SIZE)
    stream.write("\\.\n")
    return row_count


def write_count_check(stream: TextIO, table: str, expected: int) -> None:
    stream.write(
        "DO $xlstatus_count$\n"
        "DECLARE\n"
        "  actual_count BIGINT;\n"
        "BEGIN\n"
        f"  SELECT COUNT(*) INTO actual_count FROM {qident(table)};\n"
        f"  IF actual_count <> {expected} THEN\n"
        "    RAISE EXCEPTION "
        "'count mismatch after import for %: sqlite=%, postgres=%', "
        f"{qliteral(table)}, {expected}, actual_count;\n"
        "  END IF;\n"
        "END\n"
        "$xlstatus_count$;\n"
    )


def write_import_protocol(
    stream: TextIO,
    args: argparse.Namespace,
    sqlite: sqlite3.Connection,
    sqlite_tables: set[str],
    sqlite_columns: dict[str, list[str]],
    pg_columns: dict[str, list[dict[str, str]]],
    ordered: list[str],
) -> list[tuple[str, int]]:
    write_target_setup(stream, sorted(pg_columns), args.truncate)

    imported: list[tuple[str, int]] = []
    for table in ordered:
        if table not in sqlite_tables:
            continue
        available = set(sqlite_columns[table])
        columns = [meta for meta in pg_columns[table] if meta["name"] in available]
        if not columns:
            continue
        row_count = stream_table(stream, sqlite, table, columns)
        if row_count > 0:
            imported.append((table, row_count))

    for table, expected in imported:
        write_count_check(stream, table, expected)
    stream.write("COMMIT;\n")
    return imported


def run_import_transaction(
    args: argparse.Namespace,
    sqlite: sqlite3.Connection,
    sqlite_tables: set[str],
    sqlite_columns: dict[str, list[str]],
    pg_columns: dict[str, list[dict[str, str]]],
    ordered: list[str],
) -> list[tuple[str, int]]:
    cmd = psql_command(args.container, args.user, args.database)
    process = subprocess.Popen(cmd, stdin=subprocess.PIPE, text=True)
    if process.stdin is None:
        process.kill()
        process.wait()
        raise RuntimeError("failed to open psql stdin")

    try:
        imported = write_import_protocol(
            process.stdin,
            args,
            sqlite,
            sqlite_tables,
            sqlite_columns,
            pg_columns,
            ordered,
        )
        process.stdin.close()
    except BrokenPipeError as exc:
        try:
            process.stdin.close()
        except BrokenPipeError:
            pass
        returncode = process.wait()
        raise subprocess.CalledProcessError(returncode or 1, cmd) from exc
    except BaseException:
        # Closing the only psql input while the transaction is still open makes
        # PostgreSQL roll it back, including a preceding TRUNCATE.
        try:
            process.stdin.close()
        except BrokenPipeError:
            pass
        process.wait()
        raise

    returncode = process.wait()
    if returncode != 0:
        raise subprocess.CalledProcessError(returncode, cmd)
    return imported


def import_sqlite(args: argparse.Namespace) -> None:
    pg_columns = load_pg_columns(args)
    pg_tables = sorted(pg_columns)
    ordered = ordered_tables(args, pg_tables)
    sqlite, sqlite_tables, sqlite_columns = sqlite_metadata(args.sqlite_path)

    try:
        imported = run_import_transaction(
            args,
            sqlite,
            sqlite_tables,
            sqlite_columns,
            pg_columns,
            ordered,
        )
    finally:
        sqlite.close()

    for table, count in imported:
        print(f"imported\t{table}\t{count}")
    print(f"imported_tables={len(imported)}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Import an XLStatus SQLite backup into a Docker-hosted PostgreSQL database."
    )
    parser.add_argument("sqlite_path", help="path to an XLStatus SQLite .db backup")
    parser.add_argument(
        "--container",
        default="xlstatus-postgres",
        help="PostgreSQL Docker container name, default: xlstatus-postgres",
    )
    parser.add_argument("--user", default="xlstatus", help="PostgreSQL user, default: xlstatus")
    parser.add_argument(
        "--database", default="xlstatus", help="PostgreSQL database, default: xlstatus"
    )
    parser.add_argument(
        "--truncate",
        action="store_true",
        help="truncate XLStatus PostgreSQL tables before import; requires a verified backup",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    import_sqlite(args)


if __name__ == "__main__":
    main()
