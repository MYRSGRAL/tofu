# Snapshot TTL при backpressure native reply

Исходный P2: после проекции cached owner transport повторял отправку готового
Known до request deadline, который мог оказаться позже исходного snapshot expiry.

`Session::serve` сохраняет expiry исходного observation interval. Known reply
ограничивает им deadline и перед каждой попыткой отправки заново кодирует остаток
TTL. Истёкший snapshot завершает transport с Timeout; новый observation interval
из-за заполненной очереди не создаётся. Не-Known replies сохраняют request deadline.

Regression с настоящим SOCK_SEQPACKET заполняет очередь и освобождает её после
snapshot expiry, но до request deadline. RED вернул Ok вместо Timeout; GREEN
отказывает. Отдельный контроль проверяет уменьшение TTL перед незаблокированной
отправкой. Эти проверки transport используют синтетический Known frame; прежний
actual broker/cache Known roundtrip также входит в attribution suite.

Rust 1.85.1: attribution 37 PASS, clippy all-targets с `-D warnings` и fmt PASS.
Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/pr122-ttl-root/`:
RED/GREEN, source snapshots, toolchain, retained test binary и hashes.
Build использовал нормализованный Cargo.lock; package records и объединённые
dependency edges эквивалентны исходному, оба файла сохранены. Исходный lock
восстановлен без изменения pins.

Production factory остаётся fail-closed. Kernel admission, BoundTun и сквозная
production acceptance этим исправлением не принимаются. Native ABI неизменен.
