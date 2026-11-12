# P17: синхронизация presets, maintenance и ErrorContext V2

PR120 сохраняет CAS publication и matching unavailable-controller ACK из
309e730737ca9d3793d35b11a7072c0b76c37ed0. Ordinary merge parent
a5cf6cc (полный SHA a5cf6cc2a54dda72dcc8634a4aea0f53956c46d3) выполнен
коммитом 70cc834. Следующая база — main
01a639fa1c293188c4fa49e2c8d7d08227b000ff с ErrorContext V2.

При объединении сохранены typed cleanup producer, routing request_id/generation,
MaintenanceAck contexts, ready gate и остановка preset jobs перед terminal.
Backup отклоняется matching busy ACK при активном preset job. Общий unavailable
supervisor возвращает matching preset ACK, включая PresetCancel. Очистка fixtures
удаляет пустой presets-каталог перед файлами, в том числе в новом V2 backup test.
PresetPanel и CAS-контроллер не изменены относительно исходного HEAD PR120.

Проверки Rust 1.85.1, locked, jobs=1, общий with-rust-slot:

- Parent checkpoint: полный desktop cargo test PASS (лог checkpoint-tests.log).
- После V2: library 75 PASS/1 ignored, backend 44 PASS/1 ignored,
  maintenance 3 PASS, maintenance_host 1 ignored, preset_product 2 PASS.
- Clippy desktop all-targets/test-support, -D warnings: PASS.
- cargo fmt --all --check: PASS.
- RU/EN: 331 inline-пара, 101 ключ, 14 QML: PASS.
- Python diagnostic export V2: 5 PASS.

Evidence и бинарники с SHA-256 сохранены вне TTL:
/home/myrs/projects/yonik/local/validation/pr120-sync/.
Логи первоначальных compile/fixture failures сохранены рядом с финальными.
Прежние принятые Qt/Host/recovery campaigns не повторялись: core/QML не менялись.
GUI build и новый полный runtime campaign этим checkpoint не подтверждаются.

Full active mixed domain/CIDR остаётся Unsupported(RequiredRealResolution),
system_tun=false. Full P17/P11 не заявляются; Android и физические G-UI/G-POWER
отложены. Более поздние main commits с packaging/tests/docs сюда не включены.
