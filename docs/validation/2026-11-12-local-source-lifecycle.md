# Local source: Linux GUI lifecycle и remote guards

Продолжение PR134 от 2f21b25. Полный FR07 не объявляется: manual form W4,
Android, физические устройства и production TUN остаются отдельными задачами.

Backend snapshot содержит безопасный `source_local`, UI показывает локальный тип
на русском и английском. Refresh, HWID и schedule controls недоступны для local.
SourceChange не имеет отдельного штатного control в текущем Main.qml.
Backend guard отклоняет Refresh/HWID/SourceChange/schedule до сетевых действий.

`local_text_partial_approval_reopen_and_network_controls_fail_closed` использует
живой HTTP fixture: один fetch remote до local guards и второй после них.
SourceChange передаёт текущие ID/generation/request ID. Проверяются invalid_input,
сохранение revision/generation, encrypted SQLite без plaintext и побайтный original
после rename/reopen. Snapshots не содержат исходный секрет.

`local_lifecycle_native.py` запускает production QML/bridge/DesktopHost через
существующий persistence GUI example с синтетическим ключом. Обычный CLI smoke
создаёт новый ephemeral key при каждом запуске; fixture нужен только для
межпроцессного reopen. Оба закреплённых core проходят local import, RU/EN label,
disabled controls, rename/select, Connect с HTTPS probe через локальный VLESS peer,
штатный Stop, закрытие, reopen и delete. Это Local SOCKS, не TUN.

CancelLocalImport unknown-field regression включён в финальную проверку.
Старые full-import/preview/old-reader кампании не повторяются. Scheduler exclusion
доказан existing `local_v2_is_excluded_on_load_and_reload` и witness PR134/048;
данный leaf проверяет новые GUI controls и ранний schedule guard.

Evidence вне TTL: `/home/myrs/projects/yonik/local/evidence/fr07-local-lifecycle`.
Финальный manifest связывает exact HEAD/tree, production/persistence binaries,
исходники, закреплённые cores и логи hashes. Проверки: MSRV 1.85.1 GUI clippy
all-targets -D warnings, fmt, Ruff 0.17.4 check/format --check и targeted tests.
