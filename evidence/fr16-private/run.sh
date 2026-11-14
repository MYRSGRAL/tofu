#!/bin/bash
set -euo pipefail
export CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/fr16-private-821c/target
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
export CARGO_BUILD_JOBS=1
find crates -type f \( -name '*.rs' -o -name 'Cargo.toml' \) -print0 | sort -z | xargs -0 sha256sum > evidence/fr16-private/source-before.sha256
sha256sum Cargo.toml Cargo.lock >> evidence/fr16-private/source-before.sha256
cargo +1.85.1 test -p yonik-linux --lib worker:: --no-fail-fast > evidence/fr16-private/tests.log 2>&1
cargo +1.85.1 check -p yonik-linux > evidence/fr16-private/check.log 2>&1
cargo +1.85.1 clippy -p yonik-linux --all-targets -- -D warnings > evidence/fr16-private/clippy.log 2>&1
sha256sum -c evidence/fr16-private/source-before.sha256 > evidence/fr16-private/source-after.log
find "$CARGO_TARGET_DIR/debug/deps" -maxdepth 1 -type f -executable -name 'yonik_linux-*' -exec cp '{}' evidence/fr16-private/artifacts/ \;
cargo +1.85.1 clean > evidence/fr16-private/clean.log 2>&1
