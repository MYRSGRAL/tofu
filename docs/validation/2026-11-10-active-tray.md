# Активный tray: Linux T01/T02

База: `dd6962e7618d688660927bc58e2694940eba5598`.

Проверка `tools/desktop_tray_smoke.py --engine-dir <pinned engines>` использует
отдельную D-Bus session, offscreen Qt с KDE platform theme, настоящий
StatusNotifierWatcher fixture и DesktopHost. Пользовательские bus, keyring,
настройки сети и Plasma не изменяются. Нужны уже установленные dbus-python,
PyGObject и Qt KDE platform theme; установка зависимостей не выполняется.

Путь проверки: импорт локального VLESS профиля → Connect → Limited → закрытие
окна → запрос HTTP через локальный SOCKS и Xray peer → повторный запуск с тем же
data-dir → Stop через восстановленное окно → Connect → Quit с активной сессией.
Limited здесь означает отсутствие внешней health-проверки; реальную передачу
данных проверяет отдельный SOCKS HTTP запрос. Fixture peer разрешает только свой
loopback HTTP endpoint. Нет обращения к внешнему VPN серверу.

Evidence сохраняется вне TTL в `/home/myrs/projects/yonik/local/validation/`.

Открытые gates: ошибка реального cleanup с восстановлением, перезапуск Plasma,
масштабирование и энергопотребление на live desktop. Этот прогон не является
полным PASS T02 или Pxx. Для live gate нужны отдельные разрешённая сессия и
измерения; текущая задача не меняет host session.

## Результат локального прогона

- RED: исходный Qt-код не публикует hidden checkpoint при активной сессии;
  `active-tray-red3/trace.jsonl` получает hidden только на Stopping по deadline.
- GREEN: `active-tray-final/result.json` подтверждает hidden SOCKS traffic,
  IPC relaunch, Stop и Quit; SNI зарегистрирован один раз. Все девять шагов
  завершены, последний snapshot `closed=true`, listener закрыт.
- Qt production: при `cleanup_failed` открывается существующее окно с ошибкой;
  backend и политика завершения не менялись. Инжекция настоящего cleanup failure
  остаётся отдельным gate, этот пункт не считается проверенным end-to-end.
- Бинарник: `/home/myrs/projects/yonik/local/artifacts/yonik-active-tray-dd6962e/yonik-desktop`.
  SHA-256 бинарника и закреплённого Xray сохранены в
  `active-tray-final/sha256.txt` внутри evidence-каталога.

Воспроизведение из корня checkout:

```sh
python tools/desktop_tray_smoke.py /path/to/yonik-desktop \
  --engine-dir /home/myrs/projects/yonik/local/engines \
  --output /path/to/new-evidence-directory
```

Финальные проверки: Qt build `--locked` PASS, active smoke PASS, Ruff check/format
PASS, `cargo fmt --all -- --check` PASS, `git diff --check` PASS.
`cargo clippy --locked -p yonik-desktop --features gui --all-targets -j1 -- -D warnings`
останавливается на существующем `clippy::collapsible_match` в
`crates/yonik-engine/src/lib.rs:165` (вне ownership). Лог:
`/home/myrs/projects/yonik/local/validation/active-tray-clippy.log`.
