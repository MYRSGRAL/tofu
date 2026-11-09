"""Проверки изоляции, supply-chain и wire oracle native resolution pin."""

from __future__ import annotations

import json
import socket
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import resolution_pin_smoke as probe  # noqa: E402


class ResolutionPinTests(unittest.TestCase):
    def manifest(self, binary: Path) -> dict:
        records = [
            {
                "name": name,
                "feature": feature,
                "sha256": probe.digest(probe.PATCH_ROOT / name),
            }
            for name, feature in probe.PATCHES
        ]
        return dict(
            engine="sing-box",
            source_commit=probe.SOURCE_COMMIT,
            binary_sha256=probe.digest(binary),
            patch_sha256=records[0]["sha256"],
            patches=records,
            adapter_features={feature: True for _, feature in probe.PATCHES},
            strict_dns_gate=False,
        )

    def test_artifact_requires_complete_ordered_patch_chain(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "engine"
            binary.write_bytes(b"synthetic-binary")
            manifest = Path(temporary) / "manifest.json"
            original = self.manifest(binary)
            manifest.write_text(json.dumps(original))
            self.assertEqual(probe.validate_artifact(binary, manifest), binary)
            mutations = [
                {"patches": original["patches"][:-1]},
                {"patches": original["patches"][::-1]},
                {"patches": original["patches"] + [original["patches"][0]]},
                {"patch_sha256": "0" * 64},
                {"binary_sha256": "0" * 64},
                {"source_commit": "0" * 40},
                {"strict_dns_gate": True},
                {"adapter_features": {"pin_resolution": 1}},
            ]
            for mutation in mutations:
                with self.subTest(mutation=mutation):
                    manifest.write_text(json.dumps(original | mutation))
                    with self.assertRaises(ValueError):
                        probe.validate_artifact(binary, manifest)
            manifest.write_text(json.dumps(original))
            binary.write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                probe.validate_artifact(binary, manifest)

    def test_ttl_oracle_and_dns_provenance(self) -> None:
        for case in probe.CASES:
            with self.subTest(case=case.name):
                self.assertTrue(probe.expected_marker(case, None))
                self.assertTrue(probe.expected_marker(case, False))
                self.assertEqual(
                    probe.expected_marker(case, True),
                    not case.delayed
                    and case.minimum_ttl > 0
                    and case.opcode == 0
                    and not case.reserved,
                )
                a = probe.answer(case, "partial.test", 1, 1)
                self.assertEqual(a.ttl, case.ttl)
                if case.cname_ttl is not None:
                    self.assertEqual(a.cname.ttl, case.cname_ttl)
                if case.soa_minimum is not None:
                    aaaa = probe.answer(case, "partial.test", 28, 2)
                    self.assertEqual(aaaa.addresses, ())
                    self.assertEqual(aaaa.soa.minimum, case.soa_minimum)
                if case.delayed:
                    self.assertEqual(a.addresses, (probe.BLACKHOLE, probe.IPV4))
        with self.assertRaises(ValueError):
            probe.answer(probe.CASES[0], "outside.example", 1, 1)

    def test_configuration_default_is_compatible_and_udp_modes_explicit(self) -> None:
        for pin in (None, False, True):
            for udp in (None, False, True):
                config = probe.configuration(probe.CASES[0], pin, udp)
                resolve, route = config["route"]["rules"]
                self.assertEqual("pin_resolution" in resolve, pin is not None)
                self.assertEqual(resolve.get("pin_resolution"), pin)
                self.assertEqual("udp_connect" in route, udp is not None)
                self.assertEqual(route.get("udp_connect"), udp)
                self.assertEqual(config["outbounds"][0]["connect_timeout"], "1500ms")

    def test_udp_wire_parser_bounds_and_families(self) -> None:
        for family, atyp, address in (
            (socket.AF_INET, 1, probe.IPV4),
            (socket.AF_INET6, 4, probe.IPV6),
        ):
            packet = (
                b"\0\0\0"
                + bytes([atyp])
                + socket.inet_pton(family, address)
                + struct.pack("!H", 24081)
                + b"marker"
            )
            self.assertEqual(probe.decode_udp(packet), (address, 24081, b"marker"))
            for length in range(6 + (4 if atyp == 1 else 16)):
                with self.assertRaises(ValueError):
                    probe.decode_udp(packet[:length])
        for packet in (b"\0\0\1\x01" + b"x" * 10, b"\0\0\0\x03\x01x\0\1", b"x" * 4097):
            with self.assertRaises(ValueError):
                probe.decode_udp(packet)

    def test_udp_domain_reply_is_valid_and_bounded(self) -> None:
        name = b"partial.test"
        packet = (
            b"\0\0\0\x03"
            + bytes([len(name)])
            + name
            + struct.pack("!H", 24081)
            + b"marker"
        )
        self.assertEqual(probe.decode_udp(packet), ("partial.test", 24081, b"marker"))
        for length in range(5 + len(name) + 2):
            with self.assertRaises(ValueError):
                probe.decode_udp(packet[:length])
        for name in (b"", b"other.test", b"\xff" * 12):
            packet = (
                b"\0\0\0\x03" + bytes([len(name)]) + name + struct.pack("!H", 24081)
            )
            with self.assertRaises(ValueError):
                probe.decode_udp(packet)

    def test_guard_rejects_parent_and_foreign_interfaces_before_mutation(self) -> None:
        with (
            patch("network_topology_smoke.namespace", return_value=(1, 2)),
            patch.object(probe, "bounded_run") as mutate,
        ):
            with self.assertRaises(RuntimeError):
                probe.setup_namespace((1, 2))
            mutate.assert_not_called()
        with (
            patch("network_topology_smoke.namespace", return_value=(1, 3)),
            patch(
                "network_topology_smoke.ip_json",
                return_value=[{"ifname": "lo"}, {"ifname": "eth0"}],
            ),
            patch.object(probe, "bounded_run") as mutate,
        ):
            with self.assertRaisesRegex(RuntimeError, "чужие"):
                probe.setup_namespace((1, 2))
            mutate.assert_not_called()

    def test_neighbor_reset_is_guarded_and_only_uses_owned_interface(self) -> None:
        with (
            patch.object(probe, "guard_namespace", side_effect=RuntimeError("foreign")),
            patch.object(probe, "bounded_run") as mutate,
        ):
            with self.assertRaises(RuntimeError):
                probe.reset_delay_neighbor((1, 2), (1, 3))
            mutate.assert_not_called()
        with (
            patch.object(probe, "guard_namespace") as guard,
            patch.object(probe, "bounded_run", return_value=(0, b"")) as mutate,
        ):
            probe.reset_delay_neighbor((1, 2), (1, 3))
            guard.assert_called_once_with(
                (1, 2), {"lo", "pin-a", "pin-b"}, expected=(1, 3)
            )
            self.assertEqual(
                mutate.call_args.args[0], ["ip", "neigh", "flush", "dev", "pin-a"]
            )

    def test_guard_checks_identity_before_every_ip_mutation(self) -> None:
        with (
            patch.object(probe, "guard_namespace", return_value=(3, 4)) as guard,
            patch.object(probe, "bounded_run", return_value=(0, b"")) as mutate,
        ):
            self.assertEqual(probe.setup_namespace((1, 2)), (3, 4))
        self.assertEqual(guard.call_count, mutate.call_count + 1)
        self.assertTrue(
            all(
                call.kwargs.get("expected") == (3, 4)
                for call in guard.call_args_list[1:]
            )
        )
        self.assertEqual(
            mutate.call_args.args[0][-4:], ["via", "192.0.2.2", "dev", "pin-a"]
        )

    def test_launcher_is_mandatory_and_output_fresh(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = [
                "probe",
                "engine",
                "baseline",
                "--manifest",
                "manifest",
                "--output",
                str(root / "evidence"),
            ]
            with (
                patch.object(sys, "argv", args),
                patch.object(probe, "validate_artifact", return_value=root / "engine"),
                patch.object(probe, "validated_binary", return_value=root / "baseline"),
                patch.object(probe, "namespace", return_value=(9, 123)),
                patch.object(probe, "run_sandbox") as launch,
            ):
                self.assertEqual(probe.main(), 0)
                command = launch.call_args.args[0]
                for flag in (
                    "--user",
                    "--map-root-user",
                    "--keep-caps",
                    "--net",
                    "--pid",
                    "--fork",
                    "--kill-child",
                    "--mount",
                    "--mount-proc",
                    "-c",
                ):
                    self.assertIn(flag, command)
                self.assertEqual(json.loads(command[-2]), [9, 123])
                self.assertIn("runpy.run_path", command[command.index("-c") + 1])
                launch.reset_mock()
                with self.assertRaises(FileExistsError):
                    probe.main()
                launch.assert_not_called()

    def test_no_public_worker_or_namespace_override(self) -> None:
        for flag in ("--worker", "--worker-namespace", "--host-inode"):
            with (
                patch.object(
                    sys,
                    "argv",
                    [
                        "probe",
                        "engine",
                        "baseline",
                        "--manifest",
                        "manifest",
                        "--output",
                        "unused",
                        flag,
                        "1",
                    ],
                ),
                patch.object(probe, "run_sandbox") as launch,
            ):
                with self.assertRaises(SystemExit):
                    probe.main()
                launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
