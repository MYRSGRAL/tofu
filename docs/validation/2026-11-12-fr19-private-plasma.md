# FR19: функциональный lifecycle в настоящей private Plasma

Base: `e46e14157b54267069c3c80c70f6a99f08d0f38f`. Production Rust/C++/QML,
pins, capabilities и ABI не меняются. Новый `tools/desktop_plasma_smoke.py`
использует существующие Fixture/peer/step и supervisor из desktop_tray_smoke.
Мок StatusNotifierWatcher не создаётся.

Сценарий запускается в user/network namespace только с lo. Временные HOME,
XDG config/data/cache/runtime, отдельный D-Bus без service activation и
несуществующий system bus изолируют его от пользовательской Plasma.
KWin `--virtual`, software rendering, kded6 и plasmashell — настоящие системные
бинарники. Каждый процесс запускается в собственной process group;
завершение адресовано только сохранённым собственным группам. Host services,
настройки сети, keyring и desktop configuration не изменяются.

Приёмка проверяет выбранный профиль «Локальный», active_name/active_engine,
страницы server picker/settings, точные labels RU/EN через настоящий D-Bus menu,
Limited status и HTTP через SOCKS скрытого окна. Limited ожидается: health probe
направлен на закрытый loopback порт, а tunnel traffic проходит через controlled
Xray VLESS peer. Оба client cores используют свои закреплённые бинарники.

Затем supervisor завершает собственные plasmashell и kded6, проверяет исчезновение
watcher, запускает новые процессы и проверяет смену unique owner и повторную
регистрацию того же SNI. KWin и PID ядра сохраняются. Gate проверяет исходный
pidfd, повторно скрывает окно после возвращения watcher и повторяет traffic.
Stop, повторный Connect и Quit при активной сессии приходят через
`com.canonical.dbusmenu.Event`, вызывая production callbacks. Проверяются
Disconnected, reap исходного ядра, traffic новой сессии, exit 0, closed=true,
отсутствие final core PID, SOCKS listener и own IPC socket/lock.

Negative controls `tools/tests/test_desktop_plasma.py` отклоняют потерю
регистрации и настоящий terminal pidfd; live pidfd принимается. Это отдельные
чувствительные проверки continuity gate, а не подмена Plasma restart.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/fr19-plasma-lifecycle/`.
Production binary: `/home/myrs/projects/yonik/local/artifacts/fr19-plasma-lifecycle/yonik-desktop`.
Подкаталоги `*-final` содержат screenshots, Qt trace, actual menu layout,
script JSON, команды каждого процесса, stderr и итоговые owner/PID/registration.
`*-probe*` — диагностические итерации harness, не финальная приёмка.

Воспроизведение из repository root (каждый output должен быть новым):

```sh
unshare --user --map-root-user --net bash -c 'ip link set lo up; exec python tools/desktop_plasma_smoke.py /home/myrs/projects/yonik/local/artifacts/fr19-plasma-lifecycle/yonik-desktop --engine-dir /home/myrs/projects/yonik/local/engines --engine xray --language ru --output /absolute/new-output'
```

Повторить для `--engine sing-box` и `--language en` (матрица из четырёх запусков).
Runtime: KWin/PlasmaShell 6.7.5, kded6 6.30.0. Версии, source/binary hashes
сохраняются в provenance evidence.

Эта проверка относится к Linux nested functional Plasma. Физические G-UI,
G-POWER, Android и измерения производительности не входят в её claims.
Предыдущая cleanup-negative/SNI mock campaign не повторяется.

Финальная native матрица: `xray-ru-final`, `xray-en-final`,
`sing-box-ru-final`, `sing-box-en-final` — PASS, exit 0. Каждый запуск проверил
настоящий private Plasma restart, сохранение исходного ядра и hidden traffic,
SNI Stop/Connect/active Quit. `xray-ru-cleanup-final` и `sing-box-en-final`
дополнительно сохраняют `cleanup.json`: все собственные desktop/bus/app процессы
reaped, `/proc/<pid>` отсутствует (соответственно, их fd закрыты).

Проверки Python: полный discovery — 248 tests PASS; отдельные три negative
controls — PASS; Ruff check/format — PASS. Build production/gui на Rust 1.85.1
с `--offline --locked` — PASS. Build/Cargo выполняются исключительно через
`local/orchestration/with-rust-slot`, с CARGO_BUILD_JOBS=1 и собственным
flock-защищённым target вне checkout. GCC 16 выдаёт предупреждения из Qt headers;
они сохранены в build evidence.

MSRV Clippy 1.85.1 `--offline --locked -p yonik-desktop --features gui
--all-targets -- -D warnings` — PASS. Релевантная Qt приёмка выполняется на
production binary этой сборки; существующие Rust исходники не изменялись.

`cargo +1.85.1 fmt --all -- --check` — PASS. После terminal Clippy/fmt
проверены `/proc` cwd/exe/fd: ссылок на собственный transient нет.
`cargo +1.85.1 clean` выполнен через тот же slot-wrapper под flock;
собственный task-root удалён. Evidence и production binary сохранены вне TTL.
