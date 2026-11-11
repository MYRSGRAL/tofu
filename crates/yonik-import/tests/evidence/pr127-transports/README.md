# Приёмка PR 127: TLS + WS/gRPC, Linux

2026-11-12, исходная база faed18a62e0bc3cfeac7511cc8f012036ad5ee24.
Production parser/engine, зависимости, pins, capabilities и ABI не менялись.

Результат: 12/12 parser → Intent → JSON roundtrip → compile → pinned native
config-check. Матрица VLESS/VMess/Trojan × WS/gRPC × Xray/sing-box.
Проверены точные protocol, endpoint, credentials, TLS, SNI, два ALPN,
WS path и Host с портом, gRPC service. Полный compiled JSON до и после
roundtrip побайтно совпадает. См. `native.log`, hashes — `binaries.sha256`.

Негативная приёмка: mixed type/protocol (совпадающие и разные значения),
неверные типы dispatch/Host/service, неизвестные WS headers, gRPC multi_mode
не дают активируемого Intent. Debug report/server и Intent error не раскрывают
синтетические credentials и Authorization. 7/7 singbox-тестов прошли.

MSRV 1.85.1: clippy -p yonik-import -p yonik-engine --all-targets -- -D warnings
прошёл; harness отдельно собран rustc 1.85.1 -D warnings.
Полные логи, 12 JSON, 12 native logs и acceptance binary вне TTL:
`/home/myrs/projects/yonik/local/evidence/pr127-transports/`.
Воспроизводимый harness: `examples/native_transport/run.sh` (из корня repo).

Ограничения: библиотечный scope; CLI native JSON явно отвергает.
WS path с query явно запрещён существующим Intent; multi_mode не поддерживается
и отвергается, семантика не подменяется. Проверка трафика, Android и физические
G-UI/G-POWER не выполнялись. Прежние 12 TCP native checks не повторялись.
