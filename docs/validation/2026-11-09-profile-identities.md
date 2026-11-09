# Проверка идентичности серверов и избранного

Основание: `data-and-contracts.md` §1 и удобства requirements §3. Среда: CachyOS,
Intel Core Ultra 7 258V, Rust 1.85.1, системный SQLite 3.53.4.

## Контракт

Материал identity v1 включает protocol, канонический endpoint, credentials,
transport/security и все query/VMess-параметры. URI fragment и VMess `ps`
исключены. Порядок уникальных JSON/query ключей нормализован; повторные вложенные
JSON-ключи сохраняются вместе с порядком повторов, а не сворачиваются в последнее
значение. VMess endpoint/port/UUID нормализованы по разобранной модели.
Отсутствующий и явно заданный default могут различаться консервативно.

URI path включён без декодирования reserved-разделителей: `/a%2Fb` не равен
`/a/b`. Это следует из [RFC 3986 §2.2](https://www.rfc-editor.org/rfc/rfc3986#section-2.2).
Формат использует явные стабильные теги и длины полей; Rust Debug не является
wire format. SecretIdentity не раскрывается через Debug и не сохраняется в БД.

Fingerprint — HMAC-SHA256 с master key, отдельным versioned domain separator,
database ID и source ID. Одинаковый сервер в двух подписках имеет отдельные
профили/избранное. Обычный hash credentials в индексе отсутствует.

Схема v2 добавляет профили и каталог после проверки старого sentinel ключа.
Миграция v1 выполняется транзакционно; старые snapshots индексируются лениво.
Ciphertext старых sources/snapshots и master key совместимы с v1.

Snapshot CAS и reconcile каталога выполняются в одной транзакции. Совпавший
fingerprint сохраняет ID/избранное, новый получает AUTOINCREMENT ID. Дубликат
объединяется с первой parsed-записью: её имя и ordinal определяют представление,
исходный snapshot остаётся полным. Пропавшее избранное архивируется как недоступное,
возвратившееся получает прежний ID. Прочие пропавшие записи удаляются; их ID
не используются повторно. Отключение избранного у недоступной записи удаляет её.

Имя/host/port/protocol/избранное зашифрованы; AAD включает identity строки,
fingerprint, поколения/revision, ordinal и presence. Числовые счётчики/ID и
служебные положения остаются metadata SQLite. Максимум — 20 000 текущих профилей
и 20 000 избранных на источник, до 40 000 строк с архивом. Страница — 1–200
строк; поиск до 512 UTF-8 байт без controls. Поиск использует Unicode lowercase,
сортировка имени лексикографическая; это не locale-aware natural sort.

`select_profile(id, revision)` проверяет актуальность, наличие и HMAC выбранного
parsed server внутри согласованной транзакции и возвращает owned server.
Последующий refresh не меняет уже подготовленный server; повторный выбор со
старым revision отклоняется. Engine capability/config-check остаётся отдельным.

## Проверки

- На Rust 1.85.1 прошли 137 Rust-тестов и 4 doctest; fmt,
  workspace clippy `--all-targets -- -D warnings`, release build и Android
  `aarch64-linux-android` workspace compile-check. Два environment-dependent
  теста прежнего FetchHost/Secret Service остаются ignored в hermetic прогоне.
- Importer: восемь тестов identity/display name для шести протоколов,
  переименования, endpoint/credentials/options, query encoding/order, VMess
  recursive order/duplicates, legacy SS, reserved path escape, Unicode/controls
  и ограничения имени 512 байт. Raw source не изменяется.
- Repository: переименование/перестановка, дубликаты и первая запись,
  изменение identity, исчезновение/возврат/удаление избранного, source isolation,
  CAS, v1 migration/lazy index, неправильный ключ, tamper fingerprint/AAD,
  отсутствие plaintext markers и отказ будущей схемы до WAL.
- Отдельно проверены corrupt catalog counts и replay profile ciphertext от
  прежнего snapshot revision; список/выбор возвращают Corrupt.
- Пять сквозных service-тестов через локальный HTTP fixture: выбор после
  reorder, immutable прежний server, offline reopen, removed/returned favorite,
  duplicate/new credentials, query bounds и favorite, изменённый после отчёта
  и до commit. Тестовые данные вымышленные, системная сеть/keyring не менялись.

## Синтетическая нагрузка

В репозитории есть `crates/tofu-linux/examples/profile_catalog_bench.rs`.
Запуск release с 10 000 уникальных Trojan URI дал: commit с import/index —
101 мс, первая сортированная страница — 22 мс, изменение избранного — 20 мс,
поиск — 20 мс, resolve — 14 мс. Максимальный RSS отдельного процесса по
`wait4().ru_maxrss` — 15 916 KiB, БД после закрытия — 2 965 504 байта.

Дополнительно пройдены 1 000 и граничные 20 000 записей; сырые результаты
сохранены в локальном evidence. Это одиночные синтетические прогоны, не p95,
не PSS всей группы GUI/controller/engine и не закрытие G-PERF. Сценарий с
40 000 строками (включая 20 000 архивных избранных) отдельно не измерен.
Алгоритм сопоставления использует hash map, сортировка — O(n log n); БД не
делает попарное сравнение профилей. Heavy операции остаются на ограниченном
blocking pool. Небольшие страницы не требуют повторного разбора полного body.

Воспроизведение с внешним build-каталогом, удерживая lock корня:

```sh
flock -s "$tofu_build" env CARGO_TARGET_DIR="$tofu_build/target" cargo build --locked -p tofu-linux --release --example profile_catalog_bench
flock -s "$tofu_build" env TMPDIR="$tofu_build" "$tofu_build/target/release/examples/profile_catalog_bench" 10000
```

Пример создаёт только собственную временную БД с вымышленными данными и удаляет
её после закрытия; URL не загружаются, Secret Service не используется.
GUI-представление, выбор соединения в UI, планировщик и применение к сессии
остаются следующими этапами; это backend списка, а не готовое desktop-приложение.
