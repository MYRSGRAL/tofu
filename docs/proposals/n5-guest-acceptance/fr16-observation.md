# FR16: normal Host supplemental observation

SOURCE leaf не является full FR16 PASS. Основной receipt v2, parser и N5 witness
contract не изменены. Отдельный приватный артефакт `N5_FR16_HOST_LOGS_V1`
пишется в существующий caller-owned 0700 `/run/yonik-acceptance/data`:
`fr16-<core>-connected.json`, `fr16-<core>-post-stop.json`.
HOME/keypath/default backend остаются существующими. Новых входов и полномочий нет.

Каждая фаза делает ровно три normal `logs_read`: all/app/engine, after=0,
не больше 256 events и 65536 bytes JSON на страницу. Это bounded независимые
страницы, не атомарный cross-filter snapshot и не полный журнал. Counters,
точная typed Page и export остаются в приватном artifact. ID, context и typed
row сверяются между чтениями и фазами: повторный ID не может изменить контекст.
Страница и save связаны page request ID, отдельным save request ID и точным
сравнением export с bounded фактически прочитанными bytes; SHA-256 и длина
сохранены. Каждая страница экспортируется normal `logs_save` в фиксированное
имя `fr16-<core>-<phase>-<source>.txt`. NOREPLACE backend не перезаписывает
старый файл: повторная кампания требует нового подготовленного fixture.
Directory обходится по компонентам NOFOLLOW; файл проверяется на caller UID,
0600, regular type и единственную hard link. Артефакт создаётся EXCL 0600.
ACK saved_durability_unknown сохраняется буквально, не повышается до durable.

ACTUAL означает только реально полученную Host Page/save/file observation.
Native INFO классифицируется по engine row, immutable session/generation,
типизированному коду и допустимому value: Xray NativeXrayLocalDns → nativeInfo 10,
sing-box NativeSingStarted → nativeInfo 7 с centiseconds 0..8640000.
Нет такого события — UNAVAILABLE, а не expected PASS. Несоответствие наблюдателя
останавливает действие с фиксированным кодом ошибки без raw payload.

SOURCE_INVARIANT suppression не доказывает actual hostile input/load.
QML RU/EN, clipboard остаются UNAVAILABLE; headless Host не свидетельствует GUI.
Stop snapshot и bounded post-stop pages не доказывают внутренние barriers,
late-tail interleavings или worker cleanup сами по себе. Публичный receipt не
получает native details, paths, export или секреты из этого supplemental artifact.

Supplemental header содержит source_sha256 и capture_sha256 verified fixture.
Root должен сохранить эти приватные файлы вместе с source-bound ELF и исходным
N5 trusted run bundle. Этот leaf не запускает кампанию и не меняет sealed grant.
