# Общая FD-модель Xray и sing-box

База: c1c418c8bbc601d9de4ce5dee09ab4aa0bc89c58, PR 126.
В Xray перенесены ровно bootstrap_linux.go, closed_tun_linux.go и
monitor_close_linux_test.go из committed sing-box patch. Hash metadata обновлены.
Xray routing, lookup guards, callsites, pins, ABI и capabilities сохранены.
Завершение monitor держит c.mu; syscall используют SyscallConn.Control,
удерживающий runtime reference против Close/reuse. Control включает Select до
20 мс и /proc I/O: это не nonblocking и не hard wall-clock bound.

Evidence вне TTL: local/evidence/pr126-mirrored-fd в worktree PR.
RED на старом Xray closed_tun закрывает borrowed channel при удержанном mutex.
GREEN проверяет тот же тест с race detector на обеих committed копиях.
Bootstrap совпадает с isolated fixture по содержимому.

Проведены два инструментированных запуска udp/revoke Known-neighbor:
первый остановился до DNS barrier из-за неподнятого loopback в новой netns;
второй с поднятым lo прошёл. В нём фактические DNS reply, routing/guard,
replyTTL/effectiveTTL, before-dial, UDP send, peer recv/echo send, client recv
и context cancellation записаны с timestamps. A/B различены в test entrypoint,
consumer pointer записан в lookup. TTL authority не изменялась.
Второй запуск: replyTTL=20ms, effectiveTTL приблизительно 19ms, peer recv=1,
Known B получил echo. Исходный intermittent timeout НЕ воспроизведён.
Причина остаётся UNKNOWN, full Sbox acceptance не закрыта. FD-handoff не получил
отдельного наблюдения: успешный peer receive доказывает фактический egress только
для этого запуска. Следующий различающий шаг — добавить test-only наблюдения
actual socket registration и packet download WritePacket (destination/client),
связать их с A/B consumer pointer и получить именно падающую трассу.
Повторные blind runs в этой итерации не выполнялись.

Android и физические G-UI/G-POWER отложены. Production issuer не подключён.

Проверки: Xray RED ожидаемо FAIL; обе копии GREEN -race, vet, build;
реальные isolated TUN custody/constructor/pipeline с cancel/EOF/revoke/stop PASS.
Builder 18/18, scoped Ruff 0.17.4 check и format --check PASS, gofmt чист.
MSRV 1.85.1 clippy --workspace --all-targets --locked -- -D warnings и fmt PASS.
Общий Ruff format выявил пять существующих файлов вне изменения; не исправлены.
