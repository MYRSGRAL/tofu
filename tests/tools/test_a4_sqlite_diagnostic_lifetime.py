"""Регрессия: readonly диагностика не должна держать SQLite-lock до GC."""

import gc
import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path


class DiagnosticLifetime(unittest.TestCase):
    def test_context_manager_alone_blocks_exclusive_but_closing_releases(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-sqlite-lifetime-") as directory:
            path = Path(directory) / "fixture.sqlite"
            with closing(sqlite3.connect(path)) as writer:
                writer.execute("PRAGMA journal_mode=WAL")
                writer.execute("CREATE TABLE fixture (value INTEGER)")
                writer.commit()
                writer.close()
                gc.disable()
                try:
                    # Сохраняем ссылку, чтобы результат не зависел от момента GC.
                    with sqlite3.connect(
                        path.as_uri() + "?mode=ro", uri=True
                    ) as reader:
                        reader.execute("PRAGMA user_version").fetchone()
                    with closing(sqlite3.connect(path, timeout=0)) as maintenance:
                        maintenance.execute("PRAGMA locking_mode=EXCLUSIVE")
                        with self.assertRaisesRegex(sqlite3.OperationalError, "locked"):
                            maintenance.execute("BEGIN EXCLUSIVE")
                    reader.close()
                    with closing(
                        sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)
                    ) as reader:
                        reader.execute("PRAGMA user_version").fetchone()
                    with self.assertRaises(sqlite3.ProgrammingError):
                        reader.execute("SELECT 1")
                    with closing(sqlite3.connect(path, timeout=0)) as maintenance:
                        maintenance.execute("PRAGMA locking_mode=EXCLUSIVE")
                        maintenance.execute("BEGIN EXCLUSIVE")
                        maintenance.rollback()
                finally:
                    gc.enable()
