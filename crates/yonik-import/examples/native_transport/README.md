# Библиотечная приёмка TLS + WS/gRPC

`acceptance.rs` собирается отдельно через `run.sh`: зависимости workspace не
меняются, циклическая dev-зависимость import → engine не добавляется.
Запускать из корня checkout под flock собственного transient task-root и
`/home/myrs/projects/yonik/local/orchestration/with-rust-slot`, с
`CARGO_BUILD_JOBS=1`, `CARGO_TARGET_DIR` внутри task-root и `EVIDENCE_DIR` вне TTL.

Матрица: VLESS, VMess, Trojan × WS, gRPC × Xray 26.10.31, sing-box 1.14.4.
Проверяются точные SNI, ALPN (два значения), TLS, WS path и Host с портом,
gRPC service, канонический roundtrip Intent и compiled native JSON.
Каждый из 12 результатов проходит настоящий native config-check; JSON и логи
сохраняются в EVIDENCE_DIR. Credentials синтетические.

Это библиотечный gate, CLI по-прежнему явно отвергает native JSON.
Config-check не доказывает передачу трафика, Android и физические устройства
не входят в приёмку. Неизвестные WS headers и gRPC multi_mode не активируются:
негативные тесты требуют отказа Intent, вместо потери поля при компиляции.
