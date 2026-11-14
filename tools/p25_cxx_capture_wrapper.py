#!/usr/bin/python3 -I
"""Private CXX entrypoint final profile; compiler/query не выбираются через CLI."""

import os
import sys
from pathlib import Path

# Isolated Python не добавляет script directory в sys.path.
sys.path.insert(0, str(Path(__file__).parent))
from _p25_final_profile import compiler_capture, wrapper_config

if __name__ == "__main__":
    try:
        code, out, err = compiler_capture(
            wrapper_config(dict(os.environ)), sys.argv[1:], Path.cwd(), dict(os.environ)
        )
        sys.stdout.buffer.write(out)
        sys.stderr.buffer.write(err)
        raise SystemExit(code)
    except (ValueError, OSError, KeyError) as error:
        sys.stderr.write(f"Отказ CXX capture: {error}\n")
        raise SystemExit(2) from error
