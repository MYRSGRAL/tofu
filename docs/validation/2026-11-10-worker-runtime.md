# Worker runtime — 10 ноября 2026

База: PR #38 влит как `3a48c40aac5689e2d60eb73c566d374fa83b2e25` после всех
пяти зелёных jobs обоих workflows на `a660863`. Кандидат: `feat/system-worker-runtime`.
Helper/worker units не установлены и не запущены на host.

## Реализовано и проверено

- `yonik-worker --check|--run`: listening FD на stdin, UID 0 у peer, bounded
  fixed header, ровно два sealed FD, hashes/размеры/mode, CLOEXEC и RAII.
- Check проверяет version и config; Run делает exec с тем же PID, фиксированными
  аргументами/средой и config на stdin. Accepted не означает Applied.
- Конфигурация сериализуется непосредственно в учтённый memfd с incremental
  SHA-256, без промежуточного plaintext Vec. Executable копируется в sealed
  snapshot, принадлежащий worker UID, и сверяется по SHA во время копирования.
- Supervisor фиксированных systemd units: root/unique-name D-Bus binding,
  unit contract, PID/InvocationID, pending/retained transfer, cancel/recover,
  остановка socket activation до services, подтверждение пустой cgroup v2.
- Успех StartUnit/StopUnit определяется собственным JobRemoved/done, а не
  получением job path. Match подписан до вызова, отфильтрован по unit.
- Четыре service/socket и sysusers-конфигурация: отдельный worker UID,
  NET_ADMIN-only runtime, zero-capability Check и фиксированный sandbox.

## Actual gates

В собственных user/net/mount/pid namespaces прошли Xray и адаптированный
sing-box. Helper/TUN принадлежат UID 0, worker работает как UID 1. После exec
прочитан `/proc/PID/status`: UID 1, CapEff/CapBnd/CapAmb=0x1000, NoNewPrivs=1;
`/proc/PID/exe` указывает на sealed memfd. Check того же TUN-профиля запущен
с zero capabilities. Входной listening socket передан через stdin.

Профиль скомпилирован штатным compiler, сеть создана actual TUN/RPDB executors.
Peer находится за veth в соседнем netns. TCP/UDP IPv4/IPv6 проходят
TUN→VLESS→echo, marked transport обходит TUN; после остановки peer обмен
невозможен. SIGKILL клиента и обратный cleanup оставляют пустой journal.
Прошли debug и release launcher. Отдельные worker-process Check/Run gates
обоих ядер также прошли.

Private `/tmp` fixture остаётся ограниченной: debug launcher доступен другому
UID через bind mount только в собственной mount namespace, без копирования
большого ELF в tmpfs и без открытия доступа к build tree/checkout.
При root-вызове runner задаёт два UID/GID mapping; обычный пользователь
использует allocated subuid/subgid. Отсутствие mapping не заменяется UID 0.

## Проверки

- **485 Rust-тестов**, 0 failed, 19 ignored, 45 suites.
- Workspace clippy all-targets с `-D warnings`, fmt: PASS.
- **118 Python-тестов**, Ruff 0.17.4 check/format 42 tools: PASS.
- **10 namespace gates**: owned-tun, tun-config, tun-rule, public-block,
  durable-policy, service; worker-process и worker-tun для каждого ядра: PASS.
- Release `yonik-worker`: **946 816 байт**, оба actual TUN gates: PASS.
- `systemd-analyze verify`: PASS в изолированном root с настоящими unit-файлами,
  собранными executable и dependency fixtures. Это parser/load проверка,
  не запуск service с действующим systemd manager.
- YAML и bash -n четырёх bash steps: PASS. В native CI добавлен worker TUN gate.

Полные логи: `local/validation/worker/`, final-*.log,
release-nonroot-worker-tun-*.log и unit-verify.log. Использованы только
синтетические данные; пользовательские подписки/ключи/HWID не читались.

## Ревью

Два независимых review проверили FD/security и supervisor/lifecycle. Исправлены:

1. Двойное удержание serialized config Vec+memfd при одном lease. Потоковая
   сериализация устранила вторую сериализованную plaintext копию; bounded
   write/overflow/hash/seals regression прошла.
2. Асинхронный StartUnit принимался за socket readiness. Private-bus regression
   с отложенным socket bind сначала воспроизвела отказ (`activation-red.log`),
   затем прошла. Проверены failed job и фактический порядок завершения Stop jobs.

Узкий повторный review исправлений не выявил новых findings.

## Что не закрыто

Нужны установленный systemd gate с dedicated системным пользователем,
реальное дерево cgroup/fork/setsid и helper crash/restart, SystemSessionHost,
типизированный profile Prepare/Apply, controller и GUI. DNS/Fake-IP, private VPN,
LAN/reconcile и полная Linux-матрица остаются отдельными критериями.
`system_tun:false` сохранён. P12/P13 остаются частичными; общий план —
**3/26 (11,5%)**, полностью закрыть осталось 23 пункта.
