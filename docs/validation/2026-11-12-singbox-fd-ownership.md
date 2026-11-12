# Владение FD при завершении closed attach

База: ae75a3b6d0110ef5fd0f225cc38592125edd21c8, PR 126.

`monitorTUN` и `ReceiveAuthenticated` уже присутствовали в committed native patch.
Исходный race report получен новой prototype campaign, однако конфликтующие
функции не добавлены этим prototype. Production issuer остаётся неподключённым
(`runtime_hooks_connected=false`): production-достижимость этим результатом
не утверждается.

Завершение монитора теперь держит тот же mutex, что lookup и передача TUN.
Проверки pidfd, приём SCM_RIGHTS, проверка TUN и дублирование FD удерживают runtime
reference через `SyscallConn.Control`. Первичная проверка TUN предшествует передаче
в Session. ABI, pins и capabilities не менялись.

Регрессия `TestMonitorCloseWaitsForBorrower` использует настоящий SEQPACKET socket:
на старом коде под `-race` monitor закрывает borrowed FD при удержанном mutex;
на новом коде ждёт освобождения mutex, закрывает channel и отзывает Session.
Тест не использует Skip. Linux namespace acceptance включает настоящий TUN,
positive TCP/UDP echo, cancel/revoke и terminal FD/task cleanup.

Локальные evidence: `local/evidence/pr126-fd-race` (вне TTL). Сохранены RED,
полный race log, адресный повтор, vet/build, builder18, source/binary SHA256.
Исходники пакета для приёмки восстановлены из committed patch поверх предыдущего
candidate archive; бинарник является scoped candidate, не release artifact.
Полный race прогон имел один отказ `TestActualOpenedFlowsRevoke/cancel/udp`
при получении proof. Это не скрыто и не объявлено исправленным.
Общие coalescing counters, concurrent Known-neighbor и last-waiter/retry acceptance
остаются отдельными незакрытыми вопросами. Rust не менялся и Cargo не запускался;
MSRV/clippy должны оцениваться root/CI отдельно.
