# Продолжение L16: actualHost и RTNETLINK

Проверяемый production head: `0d68c46962224f90b35927918f92efca47eb1666`.
Desktop backend, QML, application.cpp/main.rs в этом срезе не менялись.

## Результаты

- Qt user path повторён сохранённым бинарником SHA256
  `8fa9616d493c3e6cfb6f2f0e02759e4597ef57a87e03f6c273fb3b28256c2915`:
  250 целей с ошибками, отдельная медленная отмена. Это не GUI-нагрузка 10k.
- Реальный batch network epoch gate прошёл за 2,27 с: 128 целей, 16 loopback
  listeners, 384 соединения первой серии. В собственном `unshare -Urn`
  выполнены `ip route add 198.51.100.0/24 dev lo` и
  `ip route add 203.0.113.0/24 dev lo`. Namespace host `4026531833`, тестовый
  `4026533529`. Реальные RTNETLINK epochs `1→2→3` под одним monitor ID.
  Завершённые и активные public results инвалидированы и очищены. Во второй
  серии при событии было completed=32, accepted=488 (384 от первой серии),
  что подтверждает незавершённую сетевую работу. Отмена третьей серии — 2 мс.
- ActualHost 10k на исходном head не прошёл: timeout ожидания snapshot,
  суммарное время 213,42 с. Запуск использовал настоящие DesktopHost и JSON
  GUI protocol, 10 000 уникальных credentials/profile IDs, 16 listeners.
  Утверждения о выполненных 30 000 соединений нет: соответствующая проверка
  не достигнута. Сохранён полный failed log.

## Причина и разрешённый bulk-срез

`start_batch` повторяет `select_profile` для каждой цели. Существующий
`resolve_profile` читает весь каталог, расшифровывает и разбирает весь snapshot
на каждый ID. Для n целей это O(n²) работы. Код одиночного resolve не менялся.

Новый private module `repository/batch_targets.rs` добавляет метод
`batch_targets(source, expected_source_generation, expected_revision, cancel)`.
Он читает текущий каталог, проверяет CAS и snapshot/profile AEAD, generation,
revision, ordinal и fingerprint каждой строки, разбирая snapshot один раз.
Возвращаются только ServerProfile и TcpProbeTarget, без ParsedServer/credentials.
При отмене возвращается None. Отмена во время ожидания DB gate не ждёт permit;
blocking операция проверяет токен до/после parse и между профилями.

Сложность O(n), n≤10 000; временно хранятся один ограниченный snapshot (≤8 MiB),
один import report (≤MAX_RECORDS), зашифрованные строки и публичные проекции.
Размер структуры проекции 136 байт (1,36 MB для 10k) без строк и allocator
capacity; это не заявление об измеренном RSS. Полные ParsedServer не копируются
в выход и освобождаются по завершении операции. Бенчмарк первых проверок:
10 000 проекций за 895 мс в debug. Это API benchmark, не actualHost gate.

Проверены equivalence с одиночным resolve и реальными TCP-сериями, stale CAS
после нового snapshot, отмена до запуска и при занятом DB gate, повреждение AEAD
профиля/snapshot. Четыре unit-теста и targeted library clippy прошли.

Для подключения нового API нужна отдельно согласованная batch-интеграция
DesktopHost. Freeze сохранён: проверка actualHost 10k остаётся открытой.
Полный P22 не объявляется завершённым. Backup/schema/общие helpers не менялись.

## Evidence и воспроизведение

Логи, PNG и provenance: `local/evidence/batch-acceptance/` (вне TTL).
`network-run.log` — failed 10k исходного head; `epoch-final.log` — успешный RTNETLINK;
`bulk-tests*.log` — API benchmark/tests; `qt/` — повтор связанного Qt binary.

Новый integration fixture `tests/batch_acceptance.rs` запускается только с
`--ignored`, внутри отдельного user/net namespace и с переменной
`YONIK_HOST_NETWORK_NS`, содержащей inode родительского host namespace.
Тест сам отклоняет host namespace и наличие других интерфейсов кроме lo.
Имена отдельных gates: `actual_10k_tcp_connections`, `real_batch_network_epoch`.
Rust-команды выполнялись через with-rust-slot, flock task root и jobs=1.
