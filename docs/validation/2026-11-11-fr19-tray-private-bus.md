# FR19: tray на отдельной D-Bus

База worktree: `90660f304e3b43e256e93eadba37f632a503ffee`.
Ownership: `application.cpp`, tray smoke, его тесты и этот документ.

Стенд использует существующий настоящий Qt DesktopHost, закреплённый Xray
`26.10.31` и loopback VLESS peer. Внешний health endpoint недоступен намеренно:
состояние `Limited`, работоспособность подтверждается HTTP через SOCKS.
Ни один положительный результат не использует поддельное ядро.

Новые шаги: Connect через QAction меню → close/hide → SOCKS HTTP → исчезновение
StatusNotifierWatcher → восстановление окна → повторное скрытие → новый watcher
на новом private D-Bus соединении → повторная SNI регистрация → IPC relaunch →
Disconnect через QAction → Connect через QAction → Quit с активной сессией.
Действия меню доступны только в существующем smoke-script пути; команды проходят
через те же QAction, что и production меню.

Исправление стенда: исключение `remove_from_connection()` больше не пропускает
quit/join GLib. RED воспроизведён исходным control-flow; GREEN проверяется
`python -m unittest discover -s tools/tests -p test_desktop_tray_cleanup.py`.
Дополнительный тест поднимает настоящий listener в потомке, игнорирующем SIGTERM,
после выхода лидера. Очистка группы закрывает listener. Лидер остаётся unreaped
до сигналов группе (`waitid/WNOWAIT`), поэтому PID группы нельзя переиспользовать
между проверкой выхода и сигналом. Очистка ограничена timeout и SIGKILL.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/fr19-tray-90660f3/`.
Бинарник вне TTL:
`/home/myrs/projects/yonik/local/artifacts/fr19-tray-90660f3/yonik-desktop`.
Сборка и Clippy: Rust 1.85.1, offline/locked, gui, jobs=1, глобальный rust slot,
BSD shared lock корня собственного transient. После сохранения бинарника —
`cargo +1.85.1 clean`.

Ограничения: физический Plasma gate OPEN (сессия заблокирована).
Gate настоящего backend `cleanup_failed` также OPEN: в текущем ownership нет
строгого feature test-support пути для инжекции отказа ресурсов. SIGSTOP ядра
не даёт требуемого отказа cleanup: SIGKILL всё равно освобождает ресурсы.
UI-ветка показа окна при `cleanup_failed` существовала на базе; её наличие
не считается end-to-end доказательством. Backend и чужие файлы не менялись.

Результаты:

- `green2/result.json`: PASS, `registered=2`, hidden traffic, watcher restart,
  relaunch, menu connect/disconnect, stop/quit. Финальный snapshot `closed=true`,
  шаг 10. Состояние активной сессии `Limited`.
- `build.log`: PASS Rust 1.85.1, Qt gui. GCC 16 выдаёт предупреждения
  `-Wsfinae-incomplete` в Qt headers; сборка успешна.
- Python discovery: 179 PASS; Ruff check/format PASS.
- `cleanup-red.log`: регрессия на исходном файле базы; GREEN — текущий suite.
- `green/`: неуспешный fixture-прогон из-за dbus exit-on-disconnect; это поведение
  явно отключено на private fixture bus. Остаточные процессы именно этого
  прогона завершены по проверенным argv. Это не положительное evidence.
- `binaries.sha256`: SHA-256 сохранённого Qt бинарника и закреплённого Xray.
  `source.sha256`/`source.diff`: исходники перед компиляцией desktop;
  итоговый Python harness сохраняется в git и `final-source.sha256`.
- `clippy.log`: PASS `cargo +1.85.1 clippy --offline --locked -p yonik-desktop
  --features gui --all-targets -j1 -- -D warnings` (Clippy 0.1.85).
- `clean.log`: удалены 6273 файла, 3.0 GiB. Активных ссылок процессов на task
  transient не осталось; пустой корень task удалён. Evidence и бинарник сохранены.
