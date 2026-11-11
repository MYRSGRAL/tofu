# P07: actual Qt simple/advanced и encrypted persistence

База `b9e5071784065c1fcb4813608d195421a146f80c`, parent PR124 `86a466dc972d0874649aabbf7c977885e71df3a4`.
Production Rust/QML/IPC/engine/wire не менялись. Тест расширяет существующий
native fixture `tray_cleanup_negative` тремя доверенными editor-only режимами.
Это не новая реализация QML или handler; используется настоящий Main.qml,
существующий bridge и DesktopHost. У fixture собственный синтетический ключ
`[43;32]`, не пользовательский секрет; normal production CLI не принимает его.

## Проверенный сценарий

`crates/yonik-desktop/tests/editor_persistence_smoke.py` вызывает существующий
Qt smoke driver. Python задаёт только действия и независимые ожидаемые snapshots;
`checkpoint` не преобразует Draft, не вычисляет ID/order/revision и не повторяет
QML handler. Все значения `when` сравниваются Qt с полным QJsonValue на равенство,
включая весь routing Draft; `ui_when` проверяет реальные QObject properties.

- Первый процесс (31 шаг): создать compound #1 с executable, двумя domain и двумя
  IPv4/IPv6 CIDR; дублировать #2, выключить #2, поднять его перед #1, задать default
  Direct через реальную routing-команду. Полный Draft ожидается в порядке [2,1].
- Переход advanced→simple проверяет неизменность Draft/revision=4 и наличие
  domain/CIDR в скрытых полях. Сохранение из simple сохраняет обе скрытые группы;
  это единственное следующее изменение revision, до 5. Последующие toggles не
  меняют Draft/revision. Invalid domain отклоняется, форма остаётся с ошибочным
  текстом, прежний saved Draft и revision=5 сохраняются.
- После shutdown второй отдельный процесс на той же encrypted SQLite DB
  (10 шагов) получает тот же Draft, ID/order/enabled/default и revision=5.
  Через Edit #1 и mode controls читает прежние значения реальных полей.
  Проверяются simple→advanced→simple, штатный shutdown/closed.
- SQLite fixture имеет закрытые права; plaintext executable/domain не присутствуют
  в файле DB. Не используется host keyring; запускаются только собственные fixtures.

## Read-only V1 reader: отдельное частичное доказательство

Доверенный Rust seed записывает фиксированный future V2 blob через существующий
EncryptedSubscriptionRepository. В нём enabled AndroidPackage #41 и disabled #42
с исходным порядком и полями. Это не реализация parser V2 или утверждение поддержки
Android. Два последующих actual Qt процесса (по 14 шагов) читают ту же БД:

- version/unknown field не распознаются текущим reader, routing_revision=null;
- edit, create, order, delete, default получают `routing_storage` с точными ACK;
- Explain получает `routing_stale`, routing_explanation=null, без fixture/default
  результата из пустого UI Draft;
- после каждого shutdown проверяются неизменные ciphertext и revision строки
  routing_draft через readonly SQLite и неизменные расшифрованные bytes через repo.

Это доказательство preservation всего unreadable blob текущим reader, **не**
закрытие обязательного leaf typed unsupported-selector preservation. Новая модель,
V1→V2 migration и отказ всех converters/compiler/SystemPolicy ещё не реализованы.
ROOT отложил production-эволюцию до единого контракта после native broker/wire handoff.
[Частный schema proposal](2026-11-11-p07-unsupported-proposal.md) оставлен для review.

## Собственные проверки и evidence

Actual: 4 запуска / 69 шагов (31+10+14+14), exit 0. Каждый процесс завершился closed.
Backend evidence: 16 snapshots основной пары, 22 snapshots read-only пары.
Команда запуска:

```sh
unshare --user --map-root-user --net env -u DBUS_SESSION_BUS_ADDRESS \
  DBUS_SYSTEM_BUS_ADDRESS=unix:path=/nonexistent dbus-run-session -- \
  python crates/yonik-desktop/tests/editor_persistence_smoke.py BINARY OUTPUT
```

Offscreen/software Qt задаёт существующий execute. Physical KDE/GPOWER и host
settings не затрагиваются. system bus намеренно недоступен. D-Bus предупреждение
об inotify `/root/.local/share/dbus-1/services` в user namespace не мешает сценарию;
нет QML ReferenceError/TypeError/binding loop. Native model source hashes:
[p07-qt-sources.sha256](p07-qt-sources.sha256).

Rust 1.85.1, --locked, jobs=1, один Cargo на target; все Cargo команды через
with-rust-slot и flock собственного transient root:

- build -p yonik-desktop --features gui,test-support --example tray_cleanup_negative: passed;
- clippy -p yonik-desktop --features gui,test-support --all-targets -- -D warnings: passed;
- fmt --all -- --check, git diff --check: passed;
- cached Ruff 0.16.6 format --check и check нового Python сценария: passed.

Первый дополнительный V2 seed завершился Storage: fixture не вызвал routing_draft()
для создания таблицы перед save. Исправлен только setup стенда; это не production
bugfix RED/GREEN. Первый actual compound/reopen сценарий уже прошёл; финальный
повтор с исправленным seed и invalid-input сценарием прошёл целиком.
CXX-Qt выдаёт предупреждения системного C++16/Qt; Rust lint errors отсутствуют.

Evidence и текущий бинарник вне TTL:
`/home/myrs/projects/yonik/local/evidence/p07-qt-99307094`.
`manifest.json` связывает hashes исходников, Cargo.lock, бинарника и rustc -Vv;
`build-final.log`, `clippy.log`, `ruff.log`, `qt-final.log`, `qt-final/result.json`,
полные synthetic backend.jsonl и v1-reader-backend.jsonl сохранены.

P07 **не complete**. A-04/A-07/A-08 в целом, actual worker/provenance, app AND domain
в TUN, native gates и P20 Apply не закрыты. `system_tun=false`; caller fixture
не verified owner. P11 errors, P05 source_change, anchor/Go/private ABI не изменялись.

После сохранения evidence проверено 0 активных fd/cwd/exe ссылок на transient root.
`cargo +1.85.1 clean` удалил 7631 файлов / 3.1 GiB; собственный task root удалён.
