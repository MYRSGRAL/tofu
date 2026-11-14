"""Readonly private X11 selection; наружу только закрытый production D2 export."""

from __future__ import annotations

import ctypes as C
import json
import os
import sys
import time
from pathlib import Path

VERIFY = Path(__file__).parent / "verification/tools"
if VERIFY.is_dir():
    sys.path.insert(0, str(VERIFY))


class Selection(C.Structure):
    _fields_ = [
        ("type", C.c_int),
        ("serial", C.c_ulong),
        ("send_event", C.c_int),
        ("display", C.c_void_p),
        ("requestor", C.c_ulong),
        ("selection", C.c_ulong),
        ("target", C.c_ulong),
        ("property", C.c_ulong),
        ("time", C.c_ulong),
    ]


class Event(C.Union):
    _fields_ = [("type", C.c_int), ("selection", Selection), ("pad", C.c_long * 24)]


def read_safe_diagnostics() -> dict:
    from check_diagnostic_export_v2 import export_metadata

    if os.environ.get("HOME") != "/home/fixture" or os.environ.get("DISPLAY") != ":99":
        raise ValueError("Только own fixture display")
    assert C.sizeof(Event) == 192
    lib = C.CDLL("libX11.so.6")
    signatures = {
        "XOpenDisplay": ([C.c_char_p], C.c_void_p),
        "XDefaultRootWindow": ([C.c_void_p], C.c_ulong),
        "XCreateSimpleWindow": (
            [
                C.c_void_p,
                C.c_ulong,
                C.c_int,
                C.c_int,
                C.c_uint,
                C.c_uint,
                C.c_uint,
                C.c_ulong,
                C.c_ulong,
            ],
            C.c_ulong,
        ),
        "XInternAtom": ([C.c_void_p, C.c_char_p, C.c_int], C.c_ulong),
        "XConvertSelection": (
            [C.c_void_p, C.c_ulong, C.c_ulong, C.c_ulong, C.c_ulong, C.c_ulong],
            C.c_int,
        ),
        "XPending": ([C.c_void_p], C.c_int),
        "XNextEvent": ([C.c_void_p, C.POINTER(Event)], C.c_int),
        "XFlush": ([C.c_void_p], C.c_int),
        "XGetWindowProperty": (
            [
                C.c_void_p,
                C.c_ulong,
                C.c_ulong,
                C.c_long,
                C.c_long,
                C.c_int,
                C.c_ulong,
                C.POINTER(C.c_ulong),
                C.POINTER(C.c_int),
                C.POINTER(C.c_ulong),
                C.POINTER(C.c_ulong),
                C.POINTER(C.POINTER(C.c_ubyte)),
            ],
            C.c_int,
        ),
        "XDestroyWindow": ([C.c_void_p, C.c_ulong], C.c_int),
        "XCloseDisplay": ([C.c_void_p], C.c_int),
        "XFree": ([C.c_void_p], C.c_int),
    }
    for name, (args, result) in signatures.items():
        function = getattr(lib, name)
        function.argtypes, function.restype = args, result
    display = lib.XOpenDisplay(None)
    if not display:
        raise RuntimeError("Own X display недоступен")
    window = 0
    try:
        window = lib.XCreateSimpleWindow(
            display, lib.XDefaultRootWindow(display), 0, 0, 1, 1, 0, 0, 0
        )
        selection = lib.XInternAtom(display, b"CLIPBOARD", 0)
        target = lib.XInternAtom(display, b"UTF8_STRING", 0)
        property_atom = lib.XInternAtom(display, b"A4_DIAGNOSTIC_READONLY", 0)
        lib.XConvertSelection(display, selection, target, property_atom, window, 0)
        lib.XFlush(display)
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            if not lib.XPending(display):
                time.sleep(0.02)
                continue
            event = Event()
            lib.XNextEvent(display, C.byref(event))
            if event.type != 31:
                continue
            if event.selection.requestor != window or event.selection.property == 0:
                raise RuntimeError("Diagnostic selection отсутствует")
            actual_type, count, after = C.c_ulong(), C.c_ulong(), C.c_ulong()
            format_bits = C.c_int()
            data = C.POINTER(C.c_ubyte)()
            status = lib.XGetWindowProperty(
                display,
                window,
                property_atom,
                0,
                16384,
                0,
                0,
                C.byref(actual_type),
                C.byref(format_bits),
                C.byref(count),
                C.byref(after),
                C.byref(data),
            )
            try:
                if (
                    status != 0
                    or format_bits.value != 8
                    or after.value
                    or count.value > 65536
                ):
                    raise RuntimeError("Diagnostic selection bounds/type отказали")
                raw = C.string_at(data, count.value)
                # Валидатор отбрасывает весь документ при любом свободном поле/секрете.
                return json.loads(export_metadata(raw))
            finally:
                if data:
                    lib.XFree(data)
        raise RuntimeError("Diagnostic selection timeout")
    finally:
        if window:
            lib.XDestroyWindow(display, window)
        lib.XCloseDisplay(display)
