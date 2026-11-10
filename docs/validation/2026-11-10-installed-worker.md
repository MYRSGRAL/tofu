# Установленные systemd workers — 10 ноября 2026

Проверены настоящие unit-файлы, dedicated user и cgroup v2 в одноразовой
Arch Linux VM. На пользовательском host helper и units не устанавливались,
его сеть и подписки не изменялись. Это приёмка worker/supervisor и TUN executor,
а не готового пользовательского подключения.

## Окружение и воспроизведение

Официальный образ: [Arch cloud image](https://mirror.pkgbuild.com/images/latest/Arch-Linux-x86_64-cloudimg.qcow2),
576 697 856 байт, SHA-256
`794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`.
Проверена detached PGP-подпись полным CI-ключом Arch-boxes с primary fingerprint
`1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`, опубликованным в
[официальном README](https://github.com/archlinux/arch-boxes).
KVM/QEMU, 2 vCPU, 2 GiB RAM, отдельные overlay/NoCloud seed/SSH-ключ;
host forwarding привязан только к loopback, без bridge с сетью host.
Guest: kernel 7.2.11-arch1-1, systemd 262.1-1-arch, cgroup v2,
системный пользователь `yonik-worker` UID/GID 969.

Установлены четыре production worker service/socket и sysusers-конфигурация.
`yonik-helper.service` заменён явно искусственным sleep-fixture для проверки
зависимости BindsTo. Production ProtectionHost, polkit и Registry через этот
fixture не проверяются. Искусственное ядро из
`tools/fixtures/systemd_worker_engine.rs` используется только для process-tree gate.

Harness-файлы: `crates/yonik-linux/tests/installed_worker.rs` и `installed_tun.rs`.
Обычный `cargo test` выполняет только безопасные guard-тесты. Для ignored gates
нужны root внутри собственной QEMU VM, systemd PID 1, cgroup v2 и root-owned
regular marker `/run/yonik-vm-gate` с mode 0600 и содержимым
`yonik-disposable-systemd-gate-v1\n`. Только после всех guards берётся общий
эксклюзивный fixture lock и разрешаются эффекты.

В VM требуются установленные worker units и launcher, fixture helper,
Python 3, iproute2 и пути `YONIK_GATE_XRAY`, `YONIK_GATE_SING_BOX`,
`YONIK_GATE_FIXTURE`, `YONIK_GATE_TOOLS`. Запускать harness с
`YONIK_SYSTEMD_VM_GATE=1`, `--ignored --test-threads=1`. Эти тесты предназначены
для уничтожаемой VM; они не являются установщиком на рабочую машину.

## Подтверждённые результаты

Все три actual gates прошли:

1. `installed_fixture_tree_and_helper_binds_to`: Stop удаляет основной процесс
   и daemonized setsid-потомка; SIGKILL helper останавливает связанные worker
   services/sockets. Recover и новый Start/Stop проходят; старые PID отсутствуют,
   cgroups пусты. Упрямый потомок, игнорирующий SIGTERM, здесь не моделируется:
   отдельное доказательство принудительной TERM→KILL эскалации ещё нужно.
2. `installed_real_cores_check_start_stop`: настоящие Xray и sing-box проходят
   Check/Run. PID/cgroup независимо сверяются через systemctl, runtime UID 969,
   CapEff/CapBnd/CapAmb = 0x1000, NoNewPrivs = 1, executable — sealed memfd.
   После Stop PID отсутствуют и cgroup пуста; повторный низкоуровневый Stop
   возвращает Stale без восстановления процесса. Пользовательский идемпотентный
   Stop остаётся обязанностью будущего session host.
3. `installed_real_cores_cross_owned_tun_and_proxy`: оба compiler-профиля
   проходят TCP/UDP IPv4/IPv6 через TUN→VLESS→echo. Production TunExecutor,
   Journal и SystemWorkerSupervisor работают в initial guest netns, peer —
   за veth в отдельной netns. Подтверждены рост TUN counters, mark bypass,
   отрицательный контроль без peer и обратный cleanup после остановки worker.

В TUN fixture точное правило IPv4 pref 1000, destination 10.0.2.0/24,
lookup main, protocol 253 сохраняет SSH управления VM. Это исключение fixture,
не реализация пользовательского LAN bypass. Оно удалено после проверки.
Emergency Drop не удаляет сеть по одному имени/приоритету: при неизвестном
состоянии сохраняет evidence и требует уничтожения VM. Штатная очистка
выполняется executor по доказанному владению ресурсами.

Финальное состояние: все пять units inactive/dead, MainPID 0; links только
lo/eth0; стандартные RPDB rules; network journal `records: []`.
VM штатно выключена, QEMU завершился с кодом 0.

Release launcher из `aeddced`: 946 816 байт, SHA-256
`6b47adf633197dad1ac39a6fe1b941b9a90c10ccb40b957f89743ade62247d52`.
Xray SHA-256 `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1`;
адаптер sing-box SHA-256
`4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.

Локальные evidence: `local/validation/systemd-vm/`, в том числе
`image.json`, `image-signature.log`, `final-installed-gates.log`,
`final-guest-state.log`, `final-network-journal.json`.

## Ревью и границы результата

Финальный локальный прогон после добавления gates и исправления lifetime journal:
**488 Rust-тестов**, 0 failed, 22 ignored, 47 suites. Workspace clippy all-targets
с `-D warnings`, fmt и diff-check прошли. Три ignored VM gates запущены отдельно
и прошли, как описано выше; остальные ignored проверки не выдаются за выполненные.
Логи: `final-workspace-tests.log`, `final-workspace-clippy.log` в каталоге evidence.

Независимые reviews worker и TUN gates выявили пробел независимой проверки
MainPID/ControlGroup, риск ожидания EOF от потомка и удаление network resources
по одному имени при emergency cleanup. Исправления включают независимую сверку,
bounded memfd output без ожидания EOF, отсутствие name-only emergency deletion
и общий fixture lock. Узкие повторные reviews findings не обнаружили.

Остаются SystemSessionHost, типизированный profile Prepare/Apply, client/controller
и GUI, readiness всего туннеля, TERM→KILL acceptance, полные crash/restart,
DNS/Fake-IP и смена сети. `system_tun:false` сохранён. P09/P12/P13/P19 остаются
частичными; общий план по полностью закрытым критериям — **3/26 (11,5%)**.
