#!/usr/bin/env bash
# Вызов только под flock task-root и with-rust-slot; каталог evidence вне TTL.
set -euo pipefail
: "${CARGO_TARGET_DIR:?Нужен собственный transient target}"
: "${EVIDENCE_DIR:?Нужен постоянный каталог evidence}"
export CARGO_BUILD_JOBS=1
cargo +1.85.1 build -p yonik-import -p yonik-engine --lib
rustfmt +1.85.1 --check --edition 2024 crates/yonik-import/examples/native_transport/acceptance.rs
rustc +1.85.1 --edition 2024 -D warnings crates/yonik-import/examples/native_transport/acceptance.rs \
  -L "dependency=$CARGO_TARGET_DIR/debug/deps" \
  --extern "yonik_import=$CARGO_TARGET_DIR/debug/libyonik_import.rlib" \
  --extern "yonik_engine=$CARGO_TARGET_DIR/debug/libyonik_engine.rlib" \
  --extern "serde_json=$(find "$CARGO_TARGET_DIR/debug/deps" -name 'libserde_json-*.rlib' -print -quit)" \
  -o "$EVIDENCE_DIR/acceptance"
"$EVIDENCE_DIR/acceptance" "$EVIDENCE_DIR"
