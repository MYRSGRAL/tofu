# Linux: согласованные snapshot пресетов (частичная FR-12)

База: eec7a56b6b03f2b022a87682f9097ce78b61fa0b. Android отложен.

`yonik_engine::presets::Candidate::parse` принимает два plain-text списка:
один DNS-суффикс на строку, один канонический CIDR на строку. Формат
`plain-domain-cidr-v1`; комментарии, URL вместо доменов, пустые строки,
неканонические CIDR отвергаются. Каждый список ограничен 4 MiB / 100 000 строк.
Версии доменов и IP обязаны совпадать и быть непустыми ограниченными токенами.
`into_rules([domain_id, ip_id], enabled)` создаёт два отдельных Direct-правила.
Bridge выделяет уникальные ID и заменяет только соответствующие preset_rules;
user_rules, LAN и private_networks хранятся отдельно.

`yonik_linux::presets::Store::open` открывает приватный каталог 0700.
`update(host, metadata, allow_local, cancel)` загружает оба ресурса через
FetchHost, без условного 304 и без выполнения скриптов; HTTP/LAN разрешён
только явным allow_local. `install` также принимает уже загруженные байты.
Metadata содержит source_id, version, format, checked_at_unix, enabled и два
ресурса с URL/revision/SHA-256. Credentials, fragment и query в URL запрещены;
metadata не реализует Debug. Ошибки не содержат URL и тела ответа.

Ожидаемые SHA и revision задаёт вызывающий код. Это контракт закреплённого
манифеста: сервер, вернувший другой выпуск, не пройдёт проверку ожидаемых SHA.
SHA из того же источника не доказывает подлинность издателя. Источник по
умолчанию не задан, kyoresuas/ru-direct не подключён. Адаптация его форматов
и доверенная доставка манифеста остаются отдельной работой.

После проверки обоих списков JSON snapshot записывается в приватный staging,
fsync, hard-link create/no-overwrite в имя SHA(snapshot), fsync каталога.
Указатель index содержит active и previous и заменяется одним rename после
fsync собственного staging. Операции записи сериализуются BSD flock каталога.
Ошибка до rename сохраняет активный указатель. Ошибка fsync после rename
возвращает PublishedDurabilityUnknown: опубликованная версия видна, её
сохранение после потери питания не гарантировано. Rollback переключает пару
active/previous; обе версии остаются на диске. Автоматический GC старых
snapshot не выполняется. Прерванные staging не участвуют в reopen.
Каталог и его родители должны находиться в управляемом приложением workspace.

Проверки: реальная временная ФС; install v1/v2, reopen/rollback; отказ mixed
revision, SHA, parse, oversize, query; private mode файлов; loopback HTTP
успешного обновления и отказ второго fetch с сохранением v1 после reopen.
Engine fixture подтверждает приоритет private/LAN/user над preset для literal IP.
RED: до экспорта API интеграционный тест завершился E0432 (presets отсутствует).
Финальный GREEN: 1 engine unit + 3 Linux integration tests прошли.
`cargo +1.85.1 clippy -p yonik-engine -p yonik-linux --all-targets -- -D warnings`
прошёл; `cargo +1.85.1 fmt --all -- --check` и `git diff --check` прошли.
Все cargo test/clippy запускались через with-rust-slot, BSD flock task root,
CARGO_TARGET_DIR вне checkout и CARGO_BUILD_JOBS=1. В clippy исправлено
чтение частичного HTTP request в fixture; новых зависимостей нет.

Это библиотечный API для следующего desktop bridge, а не закрытие полного
G-LINUX/Pxx. Внешние ядра, GUI и power-loss fault injection не запускались;
проверка доменных решений/DNS остаётся существующим domain evaluator.


## Исправление независимости active/previous

Root regression воспроизвела отказ reopen при повреждённом active, несмотря на исправный previous. Чтение index теперь проверяет форму указателей без загрузки обоих файлов; `active()` проверяет только active, `rollback()` — целевой previous. Новым резервом становится только проверенный snapshot; install при повреждённом active сохраняет исправный previous. Повреждённые исходники не изменяются. GREEN: пять preset integration tests, включая оба направления повреждения, rollback и install с сохранением исправного резерва; scoped Linux all-targets clippy и fmt прошли.

Evidence: `local/validation/pr75-corrupt-snapshot/` основного workspace. Первый запуск с переиспользованным root-cache обнаружил несовместимый старый artifact yonik-engine; после штатной инвалидации собственных engine/Linux artifacts получен behavioral RED, затем GREEN. `cargo clean -p` переставил записи Cargo.lock; парсинг TOML подтвердил идентичный граф, исходные байты lock восстановлены. Зависимости не менялись. Fault injection файловой системы и power-loss этим исправлением не доказаны.
