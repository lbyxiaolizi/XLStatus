#!/usr/bin/env python3
"""Import an XLStatus SQLite database into an initialized PostgreSQL schema.

The target PostgreSQL database must already contain the XLStatus tables. Start
the same XLStatus server version against the target PostgreSQL database once to
run migrations before using this script.
"""

from __future__ import annotations

import argparse
import csv
import io
import sqlite3
import subprocess
import sys


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
        if exc.stdout:
            sys.stderr.write(exc.stdout)
        if exc.stderr:
            sys.stderr.write(exc.stderr)
        raise


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


def qident(name: str) -> str:
    return '"' + name.replace('"', '""') + '"'


def load_pg_columns(args: argparse.Namespace) -> dict[str, list[dict[str, str]]]:
    column_sql = """
SELECT table_name, column_name, data_type, udt_name, is_nullable, COALESCE(column_default, '')
FROM information_schema.columns
WHERE table_schema = 'public'
ORDER BY table_name, ordinal_position
"""
    pg_columns: dict[str, list[dict[str, str]]] = {}
    for line in psql(args.container, args.user, args.database, column_sql, tuples=True).splitlines():
        table, column, data_type, udt_name, nullable, default = line.split("\t", 5)
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
    dependencies = {table: set() for table in pg_tables}
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
    sqlite = sqlite3.connect(f"file:{sqlite_path}?mode=ro", uri=True)
    sqlite.row_factory = sqlite3.Row
    sqlite_tables = {
        row[0]
        for row in sqlite.execute(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
        )
    }
    sqlite_columns: dict[str, list[str]] = {}
    for table in sqlite_tables:
        sqlite_columns[table] = [
            row[1] for row in sqlite.execute(f"PRAGMA table_info({qident(table)})")
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


def pg_count(args: argparse.Namespace, table: str) -> int:
    output = psql(
        args.container,
        args.user,
        args.database,
        f"SELECT COUNT(*) FROM {qident(table)};",
        tuples=True,
    ).strip()
    return int(output or "0")


def ensure_target_empty(args: argparse.Namespace, pg_tables: list[str]) -> None:
    nonempty = [(table, pg_count(args, table)) for table in pg_tables]
    nonempty = [(table, count) for table, count in nonempty if count > 0]
    if nonempty and not args.truncate:
        preview = ", ".join(f"{table}={count}" for table, count in nonempty[:10])
        raise RuntimeError(
            "target PostgreSQL tables are not empty; rerun with --truncate "
            f"to replace them after backup verification. Non-empty tables: {preview}"
        )


def import_sqlite(args: argparse.Namespace) -> None:
    pg_columns = load_pg_columns(args)
    pg_tables = sorted(pg_columns)
    ordered = ordered_tables(args, pg_tables)
    sqlite, sqlite_tables, sqlite_columns = sqlite_metadata(args.sqlite_path)

    ensure_target_empty(args, pg_tables)
    if args.truncate:
        psql(
            args.container,
            args.user,
            args.database,
            "TRUNCATE "
            + ", ".join(qident(table) for table in pg_tables)
            + " RESTART IDENTITY CASCADE;",
        )

    imported: list[tuple[str, int]] = []
    for table in ordered:
        if table not in sqlite_tables:
            continue
        available = set(sqlite_columns[table])
        columns = [meta for meta in pg_columns[table] if meta["name"] in available]
        if not columns:
            continue
        select_sql = (
            "SELECT "
            + ", ".join(qident(meta["name"]) for meta in columns)
            + " FROM "
            + qident(table)
        )
        rows = sqlite.execute(select_sql).fetchall()
        if not rows:
            continue

        buf = io.StringIO()
        writer = csv.writer(buf, lineterminator="\n")
        for row in rows:
            converted_row = []
            for meta in columns:
                converted = convert(row[meta["name"]], meta)
                converted_row.append("\\N" if converted is None else converted)
            writer.writerow(converted_row)

        copy_sql = (
            "\\copy "
            + qident(table)
            + " ("
            + ", ".join(qident(meta["name"]) for meta in columns)
            + ") FROM STDIN WITH (FORMAT csv, NULL '\\N')"
        )
        try:
            psql(args.container, args.user, args.database, copy_sql, input_text=buf.getvalue())
        except subprocess.CalledProcessError:
            sys.stderr.write(f"import failed for table {table}\n")
            raise
        imported.append((table, len(rows)))

    for table, expected in imported:
        actual = pg_count(args, table)
        if actual != expected:
            raise RuntimeError(
                f"count mismatch after import for {table}: sqlite={expected}, postgres={actual}"
            )
        print(f"imported\t{table}\t{actual}")
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
        help="truncate all public PostgreSQL tables before import; requires a verified backup",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    import_sqlite(args)


if __name__ == "__main__":
    main()
