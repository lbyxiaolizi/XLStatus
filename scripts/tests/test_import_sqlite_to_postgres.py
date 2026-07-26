from __future__ import annotations

import argparse
import importlib.util
import io
from pathlib import Path
import re
import sqlite3
import subprocess
import unittest
from unittest import mock


SCRIPT_PATH = Path(__file__).resolve().parents[1] / "import_sqlite_to_postgres.py"
SPEC = importlib.util.spec_from_file_location("import_sqlite_to_postgres", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
importer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(importer)


def args(*, truncate: bool = True) -> argparse.Namespace:
    return argparse.Namespace(
        container="test-postgres",
        user="xlstatus",
        database="xlstatus",
        truncate=truncate,
    )


def column(
    name: str,
    *,
    data_type: str = "text",
    udt_name: str = "text",
    nullable: str = "YES",
) -> dict[str, str]:
    return {
        "name": name,
        "data_type": data_type,
        "udt_name": udt_name,
        "nullable": nullable,
        "default": "",
    }


class GeneratedCursor:
    def __init__(self, total: int) -> None:
        self.total = total
        self.offset = 0
        self.fetch_sizes: list[int] = []

    def fetchmany(self, size: int) -> list[dict[str, str]]:
        self.fetch_sizes.append(size)
        count = min(size, self.total - self.offset)
        rows = [{"value": f"row-{self.offset + index}"} for index in range(count)]
        self.offset += count
        return rows

    def fetchall(self) -> None:
        raise AssertionError("streaming import must not call fetchall")


class FakeSQLite:
    def __init__(self, cursor: GeneratedCursor) -> None:
        self.cursor = cursor

    def execute(self, _sql: str) -> GeneratedCursor:
        return self.cursor


class RecordingInput:
    def __init__(self) -> None:
        self.chunks: list[str] = []
        self.closed = False

    def write(self, value: str) -> int:
        if self.closed:
            raise ValueError("write to closed input")
        self.chunks.append(value)
        return len(value)

    def close(self) -> None:
        self.closed = True

    @property
    def value(self) -> str:
        return "".join(self.chunks)


class FakeProcess:
    def __init__(self, returncode: int = 0) -> None:
        self.stdin = RecordingInput()
        self.returncode = returncode
        self.killed = False
        self.wait_calls = 0

    def wait(self) -> int:
        self.wait_calls += 1
        return self.returncode

    def kill(self) -> None:
        self.killed = True


class ImportProtocolTests(unittest.TestCase):
    def test_table_allowlist_covers_postgres_migrations(self) -> None:
        migrations = SCRIPT_PATH.parents[1] / "crates/server/migrations/postgres"
        pattern = re.compile(
            r"^\s*CREATE\s+TABLE\s+IF\s+NOT\s+EXISTS\s+([A-Za-z_][A-Za-z0-9_]*)",
            re.IGNORECASE | re.MULTILINE,
        )
        migration_tables = {
            match.group(1)
            for path in migrations.glob("*.sql")
            for match in pattern.finditer(path.read_text(encoding="utf-8"))
        }

        self.assertLessEqual(migration_tables, importer.XLSTATUS_TABLES)

    def test_postgres_metadata_ignores_unrelated_public_tables(self) -> None:
        metadata = "\n".join(
            [
                "users\tid\ttext\ttext\tNO\t",
                "customer_data\tid\ttext\ttext\tNO\t",
            ]
        )
        with mock.patch.object(importer, "psql", return_value=metadata):
            pg_columns = importer.load_pg_columns(args())

        self.assertEqual(list(pg_columns), ["users"])
        self.assertNotIn("customer_data", pg_columns)

    def test_truncate_does_not_cascade_to_unrelated_tables(self) -> None:
        stream = io.StringIO()

        importer.write_target_setup(stream, ["agents", "users"], truncate=True)

        protocol = stream.getvalue()
        self.assertIn('TRUNCATE "agents", "users" RESTART IDENTITY;', protocol)
        self.assertNotIn("CASCADE", protocol)

    def test_protocol_is_one_transaction_and_preserves_copy_text_values(self) -> None:
        sqlite = sqlite3.connect(":memory:")
        self.addCleanup(sqlite.close)
        sqlite.row_factory = sqlite3.Row
        sqlite.execute(
            "CREATE TABLE items (message TEXT, enabled INTEGER, optional_id TEXT)"
        )
        sqlite.executemany(
            "INSERT INTO items VALUES (?, ?, ?)",
            [
                ("tab\tline\nslash\\literal", 1, ""),
                (r"\N", 0, None),
                (r"\.", 1, "8aa331f0-824c-414f-87c2-6c5bcf219428"),
            ],
        )
        columns = [
            column("message", nullable="NO"),
            column("enabled", data_type="boolean", udt_name="bool", nullable="NO"),
            column("optional_id", udt_name="uuid"),
        ]
        stream = io.StringIO()

        with mock.patch.object(importer, "COPY_BATCH_SIZE", 2):
            imported = importer.write_import_protocol(
                stream,
                args(),
                sqlite,
                {"items"},
                {"items": ["message", "enabled", "optional_id"]},
                {"items": columns},
                ["items"],
            )

        protocol = stream.getvalue()
        self.assertEqual(imported, [("items", 3)])
        self.assertEqual(protocol.count("BEGIN;"), 1)
        self.assertEqual(protocol.count("COMMIT;"), 1)
        self.assertLess(protocol.index("BEGIN;"), protocol.index("TRUNCATE "))
        self.assertLess(protocol.index("TRUNCATE "), protocol.index("\\copy "))
        self.assertLess(protocol.index("\\copy "), protocol.index("SELECT COUNT(*)"))
        self.assertLess(protocol.index("SELECT COUNT(*)"), protocol.index("COMMIT;"))
        self.assertTrue(protocol.endswith("COMMIT;\n"))
        self.assertIn("tab\\tline\\nslash\\\\literal\ttrue\t\\N\n", protocol)
        self.assertIn("\\\\N\tfalse\t\\N\n", protocol)
        self.assertIn(
            "\\\\.\ttrue\t8aa331f0-824c-414f-87c2-6c5bcf219428\n",
            protocol,
        )
        self.assertIn("RAISE EXCEPTION 'count mismatch after import", protocol)

    def test_stream_table_reads_bounded_batches(self) -> None:
        cursor = GeneratedCursor(2_501)
        sqlite = FakeSQLite(cursor)
        stream = io.StringIO()

        with mock.patch.object(importer, "COPY_BATCH_SIZE", 1_000):
            count = importer.stream_table(
                stream,
                sqlite,
                "items",
                [column("value")],
            )

        self.assertEqual(count, 2_501)
        self.assertEqual(cursor.fetch_sizes, [1_000, 1_000, 1_000, 1_000])
        self.assertTrue(stream.getvalue().endswith("\\.\n"))

    def test_subprocess_receives_data_only_on_stdin(self) -> None:
        process = FakeProcess()
        cursor = GeneratedCursor(1)
        sqlite = FakeSQLite(cursor)

        with mock.patch.object(importer.subprocess, "Popen", return_value=process) as popen:
            imported = importer.run_import_transaction(
                args(),
                sqlite,
                {"items"},
                {"items": ["value"]},
                {"items": [column("value")]},
                ["items"],
            )

        self.assertEqual(imported, [("items", 1)])
        command = popen.call_args.args[0]
        self.assertNotIn("-c", command)
        self.assertNotIn("row-0", command)
        self.assertIn("row-0", process.stdin.value)
        self.assertEqual(popen.call_count, 1)
        self.assertTrue(process.stdin.closed)
        self.assertEqual(process.wait_calls, 1)

    def test_python_failure_closes_uncommitted_transaction(self) -> None:
        class BrokenValue:
            def __str__(self) -> str:
                raise ValueError("conversion failed")

        class BrokenCursor(GeneratedCursor):
            def fetchmany(self, size: int) -> list[dict[str, object]]:
                self.fetch_sizes.append(size)
                if self.offset > 0:
                    return []
                self.offset = 1
                return [{"value": BrokenValue()}]

        process = FakeProcess()
        sqlite = FakeSQLite(BrokenCursor(1))

        with mock.patch.object(importer.subprocess, "Popen", return_value=process):
            with self.assertRaisesRegex(ValueError, "conversion failed"):
                importer.run_import_transaction(
                    args(),
                    sqlite,
                    {"items"},
                    {"items": ["value"]},
                    {"items": [column("value")]},
                    ["items"],
                )

        self.assertIn("BEGIN;", process.stdin.value)
        self.assertIn("TRUNCATE ", process.stdin.value)
        self.assertNotIn("COMMIT;", process.stdin.value)
        self.assertTrue(process.stdin.closed)
        self.assertEqual(process.wait_calls, 1)

    def test_psql_failure_is_reported(self) -> None:
        process = FakeProcess(returncode=3)
        sqlite = FakeSQLite(GeneratedCursor(0))

        with mock.patch.object(importer.subprocess, "Popen", return_value=process):
            with self.assertRaises(subprocess.CalledProcessError) as raised:
                importer.run_import_transaction(
                    args(truncate=False),
                    sqlite,
                    {"items"},
                    {"items": ["value"]},
                    {"items": [column("value")]},
                    ["items"],
                )

        self.assertEqual(raised.exception.returncode, 3)
        self.assertIn("ON_ERROR_STOP=1", raised.exception.cmd)
        self.assertIn("target PostgreSQL table is not empty", process.stdin.value)


if __name__ == "__main__":
    unittest.main()
