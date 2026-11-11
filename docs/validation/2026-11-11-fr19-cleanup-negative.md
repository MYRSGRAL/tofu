# FR19: настоящий backend cleanup_failed, отдельный отрицательный Qt target

Исходный reviewed head PR103: `2f307ca8560a779a1a2e3b89a3f2ccc77633479d`.
Ветка обычным merge включает main `82d5fe7`; FR16 EventLog остаётся подключён
к LocalSession до установки отрицательного hook. Scheduler/main/worker не менялись.

Согласованный root hook существует только при `feature=test-support`.
`HostConfig::with_cleanup_fault` требует явный ephemeral key. Флаг передаётся
в LocalSession; release при fault и настоящем child принимает CleanupFailed и
возвращает HostError::Cleanup до terminate и очистки child/binary/port.
Production/default не содержит ни поля, ни builder, ни ветки release.
GUI JSON, env и замена бинарника не участвуют.

`cargo +1.85.1 build --offline --locked -p yonik-desktop --features gui,test-support
--example tray_cleanup_negative -j1` собирает отдельный отрицательный target.
Он использует оригинальные bridge.rs/application.cpp, настоящий DesktopHost,
реальные callbacks DTO и настоящий diagnostics export. Перед HostConfig выполняется
BinarySnapshot::load с закреплённым SHA-256 Xray; затем session повторяет обычную
проверку pin. Положительные gates используют production binary, не этот target.

`tools/desktop_tray_smoke.py <negative-target> --engine-dir <pinned engines>
--cleanup-negative stop|quit --output <new directory>` использует private D-Bus,
реальный loopback Xray/VLESS peer и собственную ephemeral БД. После Limited
окно скрывается; Stop/Quit приходит через QAction. Проверяется конкретный Qt
snapshot cleanup_failed/closed=false/CleanupError/can_stop=true/window_visible=true,
отсутствие выхода процесса, настоящий PID ядра и удержанный SOCKS listener.
После проверки supervisor посылает release в stdin отдельного fixture target;
trusted Rust снимает fault и вызывает настоящий повторный shutdown/Stop.

При ошибке supervisor также снимает fault, даёт bounded grace и завершает свою
Qt process group; pidfd ядра защищает от PID reuse при аварийном kill. RAII guard
fixture снимает fault и выполняет bounded final Stop при unwind. Default/Drop
LocalSession не ослаблены.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/fr19-cleanup-negative/`.
Бинарники: `/home/myrs/projects/yonik/local/artifacts/fr19-cleanup-negative/`.
`native-red` — штатный frozen Qt без fault: Quit освобождает listener, gate
удержанных ресурсов отклоняется. Это отрицательное evidence, не positive PASS.

Физический Plasma gate остаётся OPEN: live сессия заблокирована. Этот тест
не является физической проверкой трея и не закрывает весь FR19.

Результаты actual native:

- `quit-final/` и `stop-final/`: PASS, exit 0. `held.json` фиксирует конкретный
  Qt snapshot шага 8 после Close: CleanupError/cleanup_failed/closed=false,
  can_stop=true, window_visible=true, process_alive=true и listener_held=true.
  `backend.jsonl` содержит совпадающий callback настоящего DesktopHost;
  `diagnostics.json` содержит CLEANUP_FAILED/cleanup_failed.
- `backend-final.jsonl` фиксирует успешный настоящий retry после fault=false;
  финальный Qt trace — closed=true. Первый core reaped, SOCKS port закрыт.
- `positive-preserved/`: PASS на frozen production Qt PR103.
  `positive-current/`: PASS на текущем default/gui production бинарнике:
  hidden SOCKS HTTP, две SNI регистрации, смена watcher, relaunch и QAction меню.
- Rust 1.85.1: negative example build, default/gui build и constructor test PASS.
- Python discovery: 186 PASS; Ruff check/format и git diff --check PASS.
- Отдельный supervisor test использует настоящий orphan listener в отдельной
  process group после аварийного os._exit родителя. Process-local subreaper
  negative supervisor усыновляет, завершает через pidfd и reaps потомка.
- Ранние `quit-green*` — диагностические неуспешные итерации fixture/checker;
  положительные результаты только `quit-final`/`stop-final`.

`binaries.sha256` сохраняет pin Xray и SHA-256 обоих собранных бинарников.
Build evidence содержит исходный merge SHA, source.diff/source.sha256 и hash
Rust fixture перед компиляцией. Production main.rs/worker не редактировались;
изменения FR16 пришли исключительно разрешённым merge.

Финальные проверки:

- Clippy 1.85.1 `--offline --locked -p yonik-desktop --features gui,test-support
  --all-targets -j1 -- -D warnings`: PASS.
- Та же команда с `--features gui`: PASS. GCC 16 предупреждает о Qt headers
  `-Wsfinae-incomplete`; Rust/Clippy warnings отсутствуют.
- `cargo +1.85.1 clean`: удалено 8908 файлов, 4.9 GiB. Бинарники и evidence
  сохранены вне TTL; активных ссылок на transient и fixture-процессов нет,
  пустой task root удалён.
