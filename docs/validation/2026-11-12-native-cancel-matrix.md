# Ограниченная native cancellation campaign

База — `473a53b902d3a6e3ef26f286916ed1018219bc00` (PR97). Новый стенд
`crates/yonik-desktop/examples/local_lifecycle_resource.rs` использует публичный
LocalSession; DesktopHost/backend/engine contracts и pins не изменены.

Оси для каждого Xray 26.10.31 и sing-box 1.14.4:

- positive: настоящий SOCKS CONNECT и возврат уникального marker через своё ядро;
- startup_cancel: Cancellation после появления native child, обязательный exact
  `HostError::Cancelled`, затем подтверждённый Stop/Disconnected;
- traffic_cancel: свой peer получает native SOCKS greeting и подаёт Cancellation,
  pending I/O future освобождается, выполняется явный Stop;
- peer_failure: свой upstream закрыт; трафик обязан завершиться ошибкой, затем Stop.

Traffic_cancel проверяет отмену владельцем I/O с явным shutdown. Она не вызывает
LocalSession::check и не является приёмкой cancellation внутреннего health probe.

Supervisor `tools/native_cancel_matrix.py` запускает восемь ограниченных cases
в user/net/PID namespace с private proc и private D-Bus. Только loopback поднят;
маршруты host не меняются. LocalSession не использует credential repository,
поэтому ключи, keyring и постоянные credentials для этого стенда не нужны.

На каждом sample сохраняются PID/starttime, FD, threads, PSS, private dirty всех
процессов private namespace кроме baseline инфраструктуры и supervisor. После
выхода корневого стенда ни одного нового процесса оставаться не должно, включая
переподчинённых потомков и zombie. Это строгая проверка terminal process resources,
а не утверждение об отсутствии малых утечек долгоживущего controller. Peak samples
имеют период 1 ms и могут пропустить краткоживущие процессы; terminal oracle
проверяет всё namespace независимо от sampled ancestry. Сигналы посылаются только
собственным ещё не reaped Popen children: PID reuse исключён.

Отрицательный контроль запускает отдельный **собственный fixture**, удерживающий
8 MiB и FD. Oracle обязан обнаружить его, после terminate/wait ресурсы должны
исчезнуть. Утечка production core не инсценируется. Unit tests дополнительно
проверяют PID/starttime identity и реальное обнаружение/освобождение fixture.

Evidence вне TTL: `local/validation/native-cancel-matrix/` данного worktree.
Manifest сохраняет exact source HEAD и SHA256 source/binary/cores; JSON matrix
содержит raw resource samples и terminal rows, отдельный negative JSON — fixture.

Воспроизведение: собрать example Rust 1.85.1 через общий with-rust-slot, jobs=1,
CARGO_TARGET_DIR в собственном transient root под BSD flock корня. Скопировать
binary вне TTL и запустить `python tools/native_cancel_matrix.py ABS_BINARY
ABS_CORE_DIR ABS_NEW_EVIDENCE_DIR`. Сохранить evidence перед cargo clean.

TUN, DesktopHost/GUI, физический G-POWER, Android и длительный memory plateau этим
компонентным прогоном не принимаются. Старые 100 cycles не повторяются.
