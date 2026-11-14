#!/bin/bash
set -euo pipefail
export CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/fr16-followup-821c/target
export CARGO_BUILD_JOBS=1
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
out=evidence/fr16-private/followup
evidence/fr16-private/artifacts/followup-tests worker::output_transport:: --test-threads=1 --nocapture > "$out/tests.log" 2>&1
cargo +1.85.1 check --locked -p yonik-linux > "$out/check.log" 2>&1
cargo +1.85.1 clippy --locked -p yonik-linux --all-targets -- -D warnings > "$out/clippy.log" 2>&1
sha256sum -c "$out/source-before.sha256" > "$out/source-after.log"
cargo +1.85.1 fmt --all -- --check > "$out/fmt.log" 2>&1
