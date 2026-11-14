# Будущая подготовка N5

SOURCE only. В этой задаче runtime не разрешён и не запускался. API:
`prepare()` возвращает `PreparedLease`; использовать `with prepare() as lease:`
и удерживать контекст до завершения всех driver Host. `lease.receipt` содержит
наблюдённые source/profile IDs, routing revision и provenance. Закрытие seed и
повторного Host происходит до возврата. Bus/SS и их pidfd удерживаются backend.
Driver автоматически не запускается. CLI отсутствует.

Root заранее предоставляет `/usr/lib/yonik-acceptance/n5-prepare.json`:

- `schema`: `N5_PREPARATION_V1`; `caller_uid`: фактический ненулевой UID;
- `tool_source_commit` и `product_source_commit`: отдельные SHA-1 source refs;
- `profile_name`: точное однозначное имя импортируемого профиля;
- `max_seconds`: 1–3600, общий deadline операций подготовки;
- `rules`: 1–16 typed routing rules с уникальными строковыми `id`, `enabled:true`,
  пустыми `domains`/`destinations`, `action`: direct/proxy/block и одним
  `executables`: `/usr/lib/yonik-acceptance/n5-guest-driver` либо `/usr/bin/python3`;
- `pins`: отображение каждого перечисленного ниже абсолютного пути в SHA-256.

Отдельный `/usr/lib/yonik-acceptance/n5-prepare-grant.json` имеет ровно
`schema:N5_PREPARATION_GRANT_V1`,
`permission:USER_GRANTED_GUEST_PREPARATION`, `caller_uid`,
`input_sha256` (хеш точных байтов preparation JSON). Это не runtime grant.
Отсутствующие или неизвестные поля, дубликаты, несовпадения и bounds закрывают gate
до backend, mutable writes и запуска процессов. JSON ≤256 KiB; каждый pinned
артефакт ≤128 MiB. Константы NaN/Infinity запрещены.

Необходимые измеренные root-owned read-only файлы для `pins`:
`/usr/lib/yonik-acceptance/{startup-host,n5-guest-driver,n5-source.conf,product-source.tar,n5_guest_prepare.py}`,
`/usr/bin/{python3,dbus-daemon,gnome-keyring-daemon,busctl}`.
Все компоненты открываются NOFOLLOW; каталоги root-owned без group/other write,
файлы без любых write bits. Symlink executable, включая python3, недопустим:
root staging должен предоставить обычный файл по фиксированному пути.
Хеш исполняемого Python source отдельно сравнивается с tool pin.
Product ELF не приписывается commit нового инструмента. Pins — input binding,
а не доказательство соответствия ELF source или успешного acceptance.

Root заранее создаёт caller-owned 0700 `/run/user/<UID>`,
`/run/yonik-acceptance` и его `data`, `config`, `cache`, `keyring`.
Data должен быть пустым; marker, private-bus.conf, backup и socket должны
отсутствовать. Leaf не устанавливает пакеты и не меняет ownership. Требуются
обе engine в `/usr/lib/yonik/engines`, их provenance и оставшиеся native gates
проверяются отдельным acceptance driver. Подготовка не заменяет эти проверки.
Source config — subscription import, не native-profile fixture.

Private bus имеет только фиксированный unix address, без service activation
каталогов. SS запускается foreground; случайный пароль поступает только через
stdin pipe. Проверяются owner PID и unlocked default collection metadata.
Секреты и сырые snapshots не записываются. Environment строится с нуля;
никакие LD_*, user/system-bus overrides и произвольные argv не принимаются.
Команды Host ограничены import_file, backup, approve_refresh, select_profile,
edit_routing. Backup и routing используют наблюдённые revisions; reopen
проверяет IDs и полный routing draft. Нет Connect/SystemApply/probes/SQL.

Синхронный lease API требует от владельца завершить campaign и вызвать close;
deadline ограничивает операции подготовки, а не внешнюю campaign. Самостоятельного
таймера/CLI-supervisor нет. Завершение удерживает только собственные Popen/pidfd,
reap выполняется с bounds; ошибка cleanup — UNKNOWN. Mutable данные остаются
для последующего разбора, повторная подготовка поверх них закрыта.

Проверки сейчас: только unittest doubles, normal/-O и Ruff 0.17.4. Не проверены
реальные Guest DBus/SS, busctl framing, encrypted persistence и обе engines;
положительный fake flow не является runtime proof. Runtime fixture до появления
реальных IDs остаётся неполной; receipt сохраняет `runtime_grant:CLOSED`.
