# Частная FR16 codec/lifetime модель

База: 3247e6c32a5ed83ec5d812dfc1f70695d3f2b3b4. Изменения только в cfg(test) дереве worker/output_transport.

Проверяется v3 Run, 112+54R, R=0..16, ресурсный scope, закрытые роли, FD порядок 4+R, truncation/trailing/reserved/endian/unknown-version, v2 ресурсная валидация через существующий decoder. Фиксированные fixtures v1/v2/v3. Production decoder отвергает v3. PIPEFS guard использует существующий rustix fs, без зависимостей и unsafe; named FIFO — отрицательный контроль. Модель PreparedOutput создаёт две пары, fixture handle и immutable EventLog scope вместе; consume перемещает владение, поздние bytes/EOF сохраняют исходный контекст. Это не реальный Prepared/Run и не admission authority.

RED: red.log — только статическая проверка отсутствия PIPEFS guard. runtime-red.log — настоящий failing lifetime test: прямое nonblocking-чтение вернуло EAGAIN; исправлено bounded async-чтением с timeout 2 секунды.

Ограничения: Full FR16 НЕ PASS. Нет production transport/import/decoder/send/Apply/DBus/helper/exec wiring. Нет отдельного нового YNKROLE1/ABI260 golden; их исходники не изменены. Нет compile-fail доказательства отсутствия Clone/reconsume и нет запуска source-mutant кампании: отрицательные контроли изменяют входные bytes/FD, а не исходный код. PreparedOutput — unit-модель; разрешённые unit methods не выдают production permit. Factory и runtime не расширялись.

Сборка Rust 1.85.1: один transient target под корневым shared flock и with-rust-slot, jobs=1, wrappers очищены. После runtime failure выполнена инкрементальная коррекция; дополнительная compile correction: rustix предоставляет mkfifoat, не mkfifo. Во время каждой компиляции исходники не менялись. Это несколько инкрементальных прогонов, а не один успешный запуск с первой попытки. Cargo нормализовал исходный lock; compiled-Cargo.lock сохраняет использованный lock, изменение корневого lock в commit не входит. SHA manifest охватывает Rust sources, manifests и lock; включаемые binary fixtures дополнительно фиксируются в files.sha256. Локальные ELF/source архивы находятся в artifacts и не коммитятся.
