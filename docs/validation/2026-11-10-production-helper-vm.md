# Production helper: actual VM gate, 10–11 ноября 2026

Actual проверка выполнена в собственной одноразовой QEMU/KVM VM: 2 CPU, 2 GiB,
настоящие systemd, system bus, polkitd и production `yonik-helper`/`yonik-worker`.
Code-head: `024da03b71559bdb3f6735cf999438186a20ced3`, tree
`c322ec419dce2cb9d9761a4076596ceb65f6a2e3`. Последующее изменение этого отчёта
не меняет проверенные Rust-исходники, unit, policy или тестовый executable.
PR #73 остаётся draft до независимого review auth-fix. Полная приёмка P09/P12/P13,
GUI/DNS и глобальное включение system_tun этим отчётом не объявляются.

## Образ и происхождение executable

[Официальный индекс Arch](https://mirror.pkgbuild.com/images/latest/) подтвердил
релиз `Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2` (576697856 байт).
Загружены versioned qcow2, `.SHA256` и detached `.sig`. До запуска QEMU проверены
SHA256 `794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`
и GPG VALIDSIG с закреплённым primary fingerprint
`1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F`. Использован отдельный GNUPGHOME.
Образ не имел backing/data-file; только собственный overlay имел backing image.
Host packages, units, сеть и пользовательский keyring не изменялись.

Все evidence и необходимые бинарники сохранены вне TTL:
`/home/myrs/projects/yonik/local/validation/production-helper-vm/actual/`.
`candidate.json` содержит head/tree, hashes исходников и executable;
`source-024da03.tar` — exact git archive; `exact-final-state.log` подтверждает
совпадение установленных в VM executable/unit с сохранёнными артефактами.

| Артефакт | SHA256 |
| --- | --- |
| yonik-helper | `fc939fd56cacf615b6c7cdf1d75807401c2685f687ae0d3748cd35b1dc0457ef` |
| yonik-worker | `07512bbb89bcaaf22ad7941d87020c64db715bf8ec2de9305be58964f75c7467` |
| installed-helper-session | `6ed3c69f2cfb852c152c7d5febc2332e2ea25c1ccd641605021cd322807e39e3` |
| Xray 26.10.31 | `cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1` |
| helper unit | `4e3378edc331943fa961e9442b4f766052202a3802cf99e6c7b25c891e3d5922` |

Helper, worker и gate собраны Rust 1.85.1 через global two-slot wrapper,
с BSD flock собственного task root, CARGO_TARGET_DIR в нём и jobs=1.
Production исходники после auth-fix и до code-head не менялись; gate пересобран
после исправлений assertions. Финальная проверка `--offline --locked` прошла
на чистом code-head без изменения Cargo.lock. Первоначальное изменение только
сортировки lockfile старым Cargo было семантически проверено и исключено.
Xray совпадает с pinned артефактом старого стенда. Sing-box здесь не запускался.

## Подтверждённые production-дефекты: RED → GREEN

1. **Auth, commit `c5412d4`.** Настоящий Arch polkit 128-1 работает от `polkitd`,
   UID 102. Старый PeerAuthorizer требовал UID 0: trace показывает
   GetNameOwner/GetConnectionCredentials и отказ до CheckAuthorization.
   `pkcheck` при этом успешно авторизовал root тем же настоящим polkit.
   Исправление читает фиксированный `/etc/passwd`: root-owned regular file,
   NOFOLLOW/NONBLOCK, nlink=1, отсутствие group/other write, предел 1 MiB.
   Отдельная уникальная запись `polkitd` и её UID не могут иметь дубликатов.
   Поддерживается системный диапазон 1..999; иной UID/повреждение/ошибка чтения
   даёт безопасный отказ. При отсутствии отдельного аккаунта доверяется только
   root. UID не поступает от клиента/env; NSS/network lookup не используется.
   Сохраняются unique-owner fencing, проверка reply sender/type/credentials,
   замена owner и fresh authorization перед каждой операцией.
   GREEN: 15 auth-тестов, включая чужой UID, owner replacement и lookup failure;
   actual root и разрешённый непривилегированный UID успешно работают с polkitd.
   Evidence: `auth-bus.log`, `auth-daemon.log`, `polkit-identity.log`,
   `auth-final-tests.log`, `exact-allowed.log`, `exact-negative.log`.

2. **Видимость TUN, commit `7eb857a`.** Старый production unit успешно выполнял
   worker-check, но Apply отказывал: `/dev/net/tun` существовал в VM и отсутствовал
   в `/proc/helper-pid/root/dev/net/tun`. Причина — private `/dev` unit.
   Добавлены только `BindPaths=/dev/net/tun` и `DeviceAllow=/dev/net/tun rw`.
   GREEN: systemd-analyze verify exit 0; actual profile Apply и recovery проходят.
   Runtime сохраняет PrivateDevices=yes, DevicePolicy=closed, ProtectSystem=strict,
   ProtectHome=yes, RestrictNamespaces=yes, NoNewPrivileges=yes. CapEff=0x1000,
   только CAP_NET_ADMIN; `/dev/vda` и `/dev/sda` в private `/dev` не видны.
   Evidence: `profile-failure-diagnostic.log`, `tun-unit-isolation.log`,
   `exact-profile.log` и `runtime-details.log`.

Отдельно исправлены ошибки самого gate: Stop действующей public policy возвращает
`verified_rules`; `recovery_required` ожидается после restart. Для нового UID
проверяется State после собственного Apply: предварительный State другого UID
при пустом журнале прежнего владельца может консервативно вернуть `unknown`.
Неожиданный State не маскируется повторением Apply.

## Actual результаты на code-head

Fixture установил production systemd units, system-bus policy и polkit actions.
В cloud image отсутствовал polkit: внутри VM установлен подписанный официальный
пакет через pacman; host ничего не устанавливал. VM-local real polkit rule
разрешает два Yonik action только root и `yonik-vm-allowed` (UID 968), всем
остальным отказывает. Negative UID — nobody, 65534. Интерактивный agent не
используется; authorizer/helper не заменяются grant fixture.

| Сценарий | Результат и evidence |
| --- | --- |
| Public Prepare/Apply/Stop, SIGKILL actual helper, новый PID, recovery, Unlock/State | PASS, `exact-public.log` |
| Xray system-profile Prepare/Apply; живой worker; SIGKILL helper; systemd restart; journal recovery; Stop/Unlock/State | PASS, `exact-profile.log` |
| UID 65534: Prepare/Stop/Unlock требуют именно authorization error, timeout не засчитывается | PASS, `exact-negative.log` |
| UID 968: Prepare/Apply/State/Stop/Unlock через настоящий polkitd | PASS, `exact-allowed.log` |

Каждый actual ignored case запущен явно с `--ignored --exact`, результат:
**1 passed, 0 failed, 0 ignored**. `results.json` проверяет все четыре exit=0.
Сеть — синтетический VLESS peer 10.203.0.2 в собственной guest netns.
Guest management rule pref 1000 только к 10.0.2.0/24; host сеть не менялась.

Первые попытки по SSH теряли management под public block. Финальный controller
работает через собственный QEMU serial UNIX socket и VM-local root console:
проверяемая сеть не нужна для выполнения теста, evidence или rescue Unlock.
Raw zbus использует существующий IPC v1, сохраняет connection Prepare→Apply;
DBUS_SYSTEM_BUS_ADDRESS запрещён. Проверены executable через `/proc/PID/exe`,
настоящий SIGKILL, новый PID и systemd NRestarts/journal, а не только fixture task.
Старый sleep-helper/private-bus стенд не является доказательством этих результатов.

## Worker-first и foreign sentinels

Sampler читает journal и оба worker cgroup до/после чтения каждые 5 ms.
В финальном profile наблюдались 8 durable records при populated=1 у worker.
При уменьшении числа records оба cgroup уже absent/empty с обеих сторон чтения:
четыре наблюдённых снижения, включая 8→7 и 7→2. Тест отдельно требует живой
worker перед SIGKILL и пустые cgroups после recovery. `worker-first.json` и
`profile-transitions.jsonl` сохраняют порядок; sampling не доказывает каждый syscall.

До cases созданы чужие для helper nft table/chain `foreign_sentinel/retained`,
dummy link `foreign0` с 10.205.0.1/24 и RPDB rule pref 1234/protocol 253.
После всех cases исходные nft/rule/link snapshots побайтово совпали.
Итоговый journal: records=[], public_policy=null, session_recovery=null;
worker/check services и sockets inactive, managed TUN отсутствует.
Evidence: `exact-final-state.log`, `guest-transitions.tar`, `worker-first.json`.

## Проверки и очистка

Auth tests 15/15; gate host guard 1/1, четыре VM cases выше выполнены отдельно.
Clippy `--offline --no-deps -p yonik-linux --all-targets -- -D warnings`, rustfmt
и Ruff check/format проходят. Clippy с зависимостями на исходной базе выявлял
существующий collapsible_match в yonik-engine/src/lib.rs:165; engine не менялся,
полный workspace clippy PASS не заявляется.

VM штатно выключена, qemu exit=0. Свои бинарники/source/evidence сохранены,
`cargo clean` выполнен; после проверки отсутствия живых FD/cwd ссылок и получения
exclusive BSD flock удалён только собственный
`/var/tmp/codex-myrs/transient/yonik-production-helper-vm-actual`.
`cleanup-result.json` подтверждает удаление. Чужие VM, locks и Qt/private-bus
writer не затронуты. Подписанный образ — восстановимые данные, также удалён.

Независимый review auth-fix ещё требуется до перевода PR в ready; вложенное
делегирование этой implementation-задаче запрещено. Это законченная actual
проверка перечисленных сценариев Xray/kill_switch=true, не полная матрица ядер,
traffic/DNS/GUI и не разрешение глобально включать system_tun.
