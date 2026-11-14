# Будущая подготовка N5

SOURCE/mocks only; runtime CLOSED. `prepare()` возвращает `PreparedLease`.
Контекст `with prepare() as lease:` удерживает bus/SS и pidfd до окончания
внешней campaign. Seed Host и проверочный reopen закрываются до возврата.
Driver не запускается. Deadline 1–3600 секунд ограничивает подготовку;
внешняя campaign обязана явно закрыть lease. CLI/supervisor отсутствует.

## Root-sealed inputs

`/usr/lib/yonik-acceptance/n5-prepare.json` содержит ровно:

- `schema`: `N5_PREPARATION_V1`, `caller_uid`: фактический ненулевой UID;
- независимые `tool_source_commit`, `product_source_commit`: 40 hex;
- `profile_name`: точное однозначное имя импортируемого профиля;
- `max_seconds`: целое 1–3600;
- `rules`: 1–16 правил описанного ниже закрытого typed subset;
- `pins`: SHA-256 точных байтов каждого фиксированного артефакта ниже.

`/usr/lib/yonik-acceptance/n5-prepare-grant.json` содержит ровно
`schema:N5_PREPARATION_GRANT_V1`, `permission:USER_GRANTED_GUEST_PREPARATION`,
`caller_uid`, `input_sha256` точного preparation JSON. Это отдельное разрешение
подготовки, не runtime grant. Receipt всегда содержит `runtime_grant:CLOSED`.
Дубликаты/unknown fields/null вместо требуемых значений закрывают input gate.
JSON ≤256 KiB; pinned файл ≤128 MiB. Все pins/UID/grant проверяются до backend.

Фиксированные pin paths под `/usr/lib/yonik-acceptance/`:
`startup-host`, `n5-guest-driver`, `n5-python`, `n5-source.conf`,
`product-source.tar`, `n5_guest_prepare.py`,
`prereqs/dbus-daemon`, `prereqs/gnome-keyring-daemon`, `prereqs/busctl`.
Root заранее stages обычные immutable copies (например 0555 для executable,
0444 для data). Это роли соответствующих системных `/usr/bin` программ;
leaf не запускает `/usr/bin` версии и не требует менять их права/симлинки.
Системные оригиналы могут быть 0755/symlink. Ни копирования, ни установки,
ни определения путей через PATH во время подготовки нет. Внешняя guest staging
обеспечивает совместимые dynamic loader/libraries и runtime зависимости SS.

NOFOLLOW действует для всех компонентов, каталоги root-owned без group/other
write, файлы root-owned без любых write bits. Executable pins независимы от
source archive и tool pin. Исполняемый Python source сверяется с tool pin.
Это binding входов, не доказательство ELF↔source соответствия или acceptance.
Обе engine должны быть заранее staged в `/usr/lib/yonik/engines`; их provenance
и native authority остаются отдельными driver gates.

## Раскладка mutable state

`/run/yonik-acceptance` обязан быть root:root без group/other write — здесь
sealed driver читает root launch controls через `root_file_since`.
Root заранее создаёт caller-owned 0700:

- `/run/user/<UID>`;
- `/run/yonik-acceptance/data` — фиксированный путь driver;
- `/run/yonik-acceptance/prepare` и его `config`, `cache`, `keyring`, `data`.

Root **заранее** bind-mounts `prepare/data` на тот же каталог, что и внешний
`data`. Leaf проверяет NOFOLLOW directories и совпадение device/inode обеих
точек. Это необходимый staging prerequisite: startup_host принимает один root
и неизменно использует root/data. HOME и root argv здесь равны `.../prepare`;
marker, private-bus.conf, backup и XDG state лежат внутри caller subtree.
Leaf не монтирует, не chown, не ставит пакеты. Data/keyring изначально пусты;
marker/config/backup/socket отсутствуют. Сохранённые mutable данные не удаляются.

## Routing и production interop

Проверены исходники `editor.rs::RuleInput`, `Draft::put`,
`yonik-engine/src/transport.rs::TransportInput`, Host snapshots/dispatch,
`n5_guest_driver/sealed.rs::{data,controls,root_file_since}`.
Rule содержит ровно `id` (int, не bool, 1..9007199254740991), `enabled:true`,
один `executables` (фиксированный driver либо фиксированный `n5-python`),
`domains:[]`, `destinations` (1–16 canonical IP CIDR), `action`
(direct/proxy/block), `transport` с обоими полями:
`transport_protocols` (tcp/udp, без повторов), `destination_ports`
(1–16 непересекающихся диапазонов `[first,last]`, 1..65535).

Узкий subset исключает произвольные пути и broad rules, но позволяет выразить
матрицу driver application×transport×action на разных destination/port.
Root DATA должен задать фактическую матрицу, включая allow-control для block;
leaf не выдумывает endpoints, flows или IDs и не доказывает полноту кампании.
Host сам повышает draft до V3 и добавляет platform_sources при typed edit.
Rule передаётся без преобразования; backup/routing используют observed revisions.
Nullable maintenance/routing ACK ожидаются безопасно. Reopen проверяет actual
IDs и весь сохранённый draft, включая transport. Connect/SystemApply/SQL/probes
и ключевые overrides не используются.

## Cleanup и предел проверки

Child остаётся в backend до успешного close. Stream-close и signal ошибки
не мешают bounded wait→TERM→wait→KILL→wait. До подтверждённого reap pidfd
удерживается; при неудаче UNKNOWN сохраняет ownership. Sticky UNKNOWN не
становится success после второго close, даже если последующий reap удался.
Cleanup exception имеет `owner` (Child/Backend) для повторного cleanup/разбора;
lease при UNKNOWN не получает `closed=True`. Нельзя трактовать UNKNOWN как PASS.

Проверки — pure mocks: nullable ACK, IDs/bounds, typed flow matrix, reopen,
ошибки stdin/stdout/signal/wait/fd, удержание ownership и повторный close.
Ни реальные DBus/SS/Host/DB, ни сервисы/VM/ELF/build не запускались.
Source interop сверка не заменяет будущий granted guest acceptance, включая
реальную bind-mount раскладку, busctl framing и encrypted persistence.
