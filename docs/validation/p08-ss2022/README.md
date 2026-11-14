# P08: positive SS2022 witness

База: `3247e6c32a5ed83ec5d812dfc1f70695d3f2b3b4`. RED в `red.json` — отсутствие positive witness, не ожидаемый отказ engine. GREEN: импорт synthetic URI с Base64 от 16 байт `0123456789abcdef`, `yonik_engine::compile`, exact method/password JSON, Xray 26.10.31 `run -test` и sing-box 1.14.4 `check`, оба exit 0. Wrong-length (15 bytes) отвергнут обоими; SS2022 chacha с 32-byte key отвергнут Xray. Старый invalid-key test сохранён.

Воспроизведение: `python3 tools/check_ss2022.py --compiler /path/to/yonik-compile --xray /path/to/xray --sing-box /path/to/sing-box --evidence /path/to/evidence`.

Retained tofu compile 370dbfa не переиспользован: source closure отличается; старый singbox-p04 helper имеет другой CLI/API. Выполнена одна focused build Rust 1.85.1: root task-directory flock, shared with-rust-slot, jobs=1, без RUSTC wrappers. ELF, исходный tar, pre/post SHA256 Rust closure и полные build logs сохранены вне TTL: `/home/myrs/projects/yonik/local/evidence/p08-ss2022-3247`. Rust source hashes до/после совпадают. Отклонение freeze-протокола: Python entrypoint добавлен во время Rust build; компилируемые исходники не менялись. Cargo переставил workspace entries lockfile; diff сохранён локально, исходный lockfile восстановлен, зависимости не менялись.

Проверки: compiler integration 12/12; cargo fmt; clippy yonik-engine all-targets -D warnings; Ruff обоих Python файлов. Cargo clean завершён, пустой task-каталог удалён. Production compiler не менялся. Трафик, VM, TUN, probes и runtime DNS не запускались; config-check не доказывает handshake/runtime interoperability. Синтетические JSON и ключ публичные. Версии и SHA256 ELF записаны в artifacts.json; это проверка локальных pinned ELF, не повторный аудит их сборки.
