"""Конечный gate 14 границ exact source172; не свидетельство принятия в main."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import unittest
from pathlib import Path

from tools.tests.test_p11_acceptance_closure import CONTEXTS, ROOT

LEDGER = ROOT / "docs/validation/p11-integrated-candidate.json"


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def fresh(blobs: dict[str, str], root: Path = ROOT) -> bool:
    return all(
        git("hash-object", str(root / path)) == blob for path, blob in blobs.items()
    )


class IntegratedCandidate(unittest.TestCase):
    def setUp(self) -> None:
        self.ledger = json.loads(LEDGER.read_text())

    def test_exact_source_and_working_tree_freshness(self) -> None:
        source = self.ledger["source"]
        self.assertEqual(source, "6f45329b2e0a9b1659232fd4558d75308a6bc6c7")
        self.assertTrue(fresh(self.ledger["source_blobs"]))
        if git("rev-parse", "--is-shallow-repository") == "true":
            return  # CI проверяет file hashes; ancestry требует полной истории.
        for path, blob in self.ledger["source_blobs"].items():
            self.assertEqual(git("rev-parse", f"{source}:{path}"), blob)
        subprocess.run(
            ["git", "merge-base", "--is-ancestor", source, "HEAD"], cwd=ROOT, check=True
        )
        damaged = dict(self.ledger["source_blobs"])
        damaged[next(iter(damaged))] = "0" * 40
        self.assertFalse(fresh(damaged), "Устаревшее evidence должно отклоняться")

    def test_original_12_plus_import_startup_exactly_once(self) -> None:
        original = self.ledger["original_inventory"]
        historical = json.loads(
            (ROOT / "docs/validation/p11-acceptance-closure.json").read_text()
        )
        self.assertEqual(original, historical["original_inventory"])
        expected = {r["id"] for r in original["boundaries"] + original["separate"]}
        rows = self.ledger["boundaries"]
        self.assertEqual(len(rows), 14)
        self.assertEqual({r["id"] for r in rows}, expected)
        for row in rows:
            with self.subTest(boundary=row["id"]):
                self.assertIn(row["file"], self.ledger["source_blobs"])
                self.assertTrue(row["publication"])
                self.assertTrue(row["representative_contract"])
                for reason, identity in row["representative_contract"].items():
                    self.assertEqual(CONTEXTS[reason], identity)
        self.assertIs(self.ledger["P11_main_closed"], False)

    def test_all_typed_producer_references_have_final_contract(self) -> None:
        # Дополняет representative rows всеми Cause-ссылками четырёх producers.
        for name in (
            "desktop_schedule",
            "source_failure",
            "maintenance_job",
            "backend_startup",
        ):
            source = (
                (ROOT / f"crates/yonik-desktop/src/{name}.rs")
                .read_text()
                .split("#[cfg(test)]", 1)[0]
            )
            causes = set(re.findall(r"Cause::([A-Za-z0-9_]+)", source))
            self.assertTrue(causes)
            for cause in causes:
                reason = re.sub(r"(?<!^)(?=[A-Z])", "_", cause).lower()
                self.assertIn(reason, CONTEXTS, (name, cause))

    def test_reused_producer_evidence_is_bound_and_not_runtime_claim(self) -> None:
        evidence = self.ledger["producer_evidence"]
        binding_path = ROOT / evidence["binding"]
        # Локальный полный log не обязателен для CI; source binding проверяется Git.
        if git("rev-parse", "--is-shallow-repository") == "true":
            self.skipTest(
                "Повторное использование Rust evidence требует полной Git history"
            )
        for path in self.ledger["source_blobs"]:
            if path.startswith("crates/yonik-desktop/"):
                self.assertEqual(
                    git("rev-parse", f"{evidence['source']}:{path}"),
                    self.ledger["source_blobs"][path],
                )
        if binding_path.exists():
            binding = json.loads(binding_path.read_text())
            self.assertEqual(binding["source_head"], evidence["source"])
            log = (ROOT / evidence["log"]).read_bytes()
            self.assertEqual(hashlib.sha256(log).hexdigest(), evidence["log_sha256"])
            for test in (
                "fetch_causes_strict_v2_generation_fence",
                "probe_producer_exports_typed_failure_and_rejects_stale_generation",
                "boundaries_preserve_admission_and_replacement_failures",
                "join_failure_keeps_owner_context_and_unconfirmed_drain",
                "thread_start_failure_publishes_closed_context_before_return",
                "manual_source_failure_сохраняет_enum_до_публикации",
                "actual_backup_conflict_is_bound_to_ack_and_v1_is_unchanged",
            ):
                self.assertIn((test + " ... ok").encode(), log)
