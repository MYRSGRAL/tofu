# Разрешённый bounded kernel observation adapter

Пользователь явно разрешил 12 ноября 2026 узкий observation adapter из
local/orchestration/proposals/kernel-enforcement-probe/adapter.patch.
В yonik-nsfs добавлен safe observe() с Report errno; приватный unsafe модуль
выполняет ровно unshare(-1), setns(-1,0), clone3(NULL,0). Caller не задаёт
syscall number, flags, FD или pointer. Архитектурная allowlist x86_64/aarch64;
остальные возвращают Unsupported. Общая библиотека ограничена Linux.

Исходный проверенный patch применён без изменения поведения, затем rustfmt.
Новых зависимостей, pins, capabilities или worker/native ABI нет. Gate mint
и verified_helper_bootstrap остаются закрытыми. EPERM/ENOSYS не устанавливают
происхождение фильтра и не доказывают блокировку допустимых аргументов.

Rust 1.85.1 offline/locked: yonik-nsfs all-targets 2 PASS, clippy all-targets
-D warnings и fmt PASS. Повтор retained binary показал errno и неизменность
net/pid/mnt namespace identities. Runtime evidence только x86_64; aarch64
здесь не запускался. Предыдущие external controls сохранены в proposal и
не выдаются за повторную кампанию нового binary.

Evidence: /home/myrs/projects/yonik/local/evidence/kernel-observation-approved/.
Production admission, canonical unit/executable/invocation binding и Journal /
BoundTun ownership интегрируются отдельными проверяемыми срезами. Три TUN ioctl
имеют отдельное разрешение пользователя и этим изменением не добавлены.
