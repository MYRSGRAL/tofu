"""Проверка DNS wire helpers, от которых зависит достоверность native gate."""

from __future__ import annotations

import struct
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from dns_fixture import (  # noqa: E402
    Cname,
    DnsFixture,
    Reply,
    Soa,
    decode_name,
    dns_query,
    make_response,
    question_bytes,
    response_addresses,
)


def query(name: str = "fixture.test", kind: int = 1) -> bytes:
    return (
        b"\x12\x34"
        + struct.pack("!HHHHH", 0x0100, 1, 0, 0, 0)
        + question_bytes(name, kind)
    )


class DnsWireTests(unittest.TestCase):
    def test_complete_ipv4_and_ipv6_answers(self) -> None:
        for kind, address in [(1, "198.51.100.10"), (28, "2001:db8::10")]:
            packet = make_response(query(kind=kind), Reply((address,), ttl=1))
            ttls: list[int] = []
            self.assertEqual(
                response_addresses(packet, b"\x12\x34", "fixture.test", kind, ttls),
                [address],
            )
            self.assertEqual(ttls, [1])
        packet = make_response(query(), Reply())
        self.assertEqual(response_addresses(packet, b"\x12\x34", "fixture.test", 1), [])

    def test_error_truncation_identity_and_trailing_bytes_are_rejected(self) -> None:
        valid = make_response(query(), Reply(("198.51.100.10",)))
        truncated = valid[:2] + struct.pack("!H", 0x8380) + valid[4:]
        for packet in [
            make_response(query(), Reply(rcode=2)),
            truncated,
            valid[:-1],
            valid + b"extra",
        ]:
            with self.assertRaises(ValueError):
                response_addresses(packet, b"\x12\x34", "fixture.test", 1)
        with self.assertRaises(ValueError):
            response_addresses(valid, b"\x00\x00", "fixture.test", 1)
        with self.assertRaises(ValueError):
            response_addresses(valid, b"\x12\x34", "other.test", 1)

    def test_compression_loops_and_long_names(self) -> None:
        with self.assertRaises(ValueError):
            decode_name(b"\xc0\x00", 0)
        name = ".".join(["a"] * 127)
        packet = make_response(query(name), Reply(("198.51.100.10",)))
        self.assertEqual(
            response_addresses(packet, b"\x12\x34", name, 1), ["198.51.100.10"]
        )
        with self.assertRaises(ValueError):
            question_bytes("a" * 64 + ".test", 1)

    def test_family_and_resource_count_bounds(self) -> None:
        with self.assertRaises(ValueError):
            make_response(query(), Reply(("2001:db8::1",)))
        with self.assertRaises(ValueError):
            make_response(query(), Reply(tuple(["198.51.100.10"] * 65)))
        packet = make_response(query(), Reply())
        invalid = packet[:6] + struct.pack("!H", 65) + packet[8:]
        with self.assertRaises(ValueError):
            response_addresses(invalid, b"\x12\x34", "fixture.test", 1)

    def test_cname_owner_and_ttls_are_preserved(self) -> None:
        packet = make_response(
            query(), Reply(("198.51.100.10",), ttl=30, cname=Cname("target.test", 1))
        )
        ttls: list[int] = []
        self.assertEqual(
            response_addresses(packet, b"\x12\x34", "fixture.test", 1, ttls),
            ["198.51.100.10"],
        )
        self.assertEqual(ttls, [1, 30])
        self.assertEqual(struct.unpack_from("!H", packet, 6)[0], 2)
        loop = make_response(query(), Reply(cname=Cname("fixture.test", 1)))
        with self.assertRaises(ValueError):
            response_addresses(loop, b"\x12\x34", "fixture.test", 1)
        for ttl in (-1, 2**32):
            with self.assertRaises(ValueError):
                make_response(query(), Reply(cname=Cname("target.test", ttl)))

    def test_soa_keeps_wire_ttl_and_minimum_separate(self) -> None:
        packet = make_response(query(), Reply(soa=Soa("test", 30, 5)))
        self.assertEqual(struct.unpack_from("!H", packet, 8)[0], 1)
        _, offset = decode_name(packet, 12)
        owner, offset = decode_name(packet, offset + 4)
        self.assertEqual(owner, "test")
        kind, qclass, ttl, size = struct.unpack_from("!HHIH", packet, offset)
        self.assertEqual((kind, qclass, ttl), (6, 1, 30))
        payload = offset + 10
        _, next_offset = decode_name(packet, payload)
        _, next_offset = decode_name(packet, next_offset)
        self.assertEqual(
            struct.unpack_from("!IIIII", packet, next_offset), (1, 60, 30, 3600, 5)
        )
        self.assertEqual(next_offset + 20, payload + size)
        self.assertEqual(response_addresses(packet, b"\x12\x34", "fixture.test", 1), [])
        with self.assertRaises(ValueError):
            make_response(query(), Reply(soa=Soa("test", 30, -1)))

    def test_invalid_opcode_and_reserved_bit_are_available_negative_controls(
        self,
    ) -> None:
        for reply in (
            Reply(("198.51.100.10",), opcode=2),
            Reply(("198.51.100.10",), reserved=True),
        ):
            packet = make_response(query(), reply)
            flags = struct.unpack_from("!H", packet, 2)[0]
            self.assertEqual((flags >> 11) & 15, reply.opcode)
            self.assertEqual(bool(flags & 0x40), reply.reserved)
            with self.assertRaises(ValueError):
                response_addresses(packet, b"\x12\x34", "fixture.test", 1)
        for opcode in (-1, 16):
            with self.assertRaises(ValueError):
                make_response(query(), Reply(opcode=opcode))

    def test_udp_fixture_records_question_and_stops(self) -> None:
        with DnsFixture(
            lambda _name, kind, _index: Reply(("127.0.0.1",)) if kind == 1 else Reply(),
            port=0,
        ) as fixture:
            self.assertEqual(
                dns_query("fixture.test", port=fixture.port), ["127.0.0.1"]
            )
            self.assertEqual(dns_query("fixture.test", qtype=28, port=fixture.port), [])
        self.assertEqual([record["qtype"] for record in fixture.records], [1, 28])


if __name__ == "__main__":
    unittest.main()
