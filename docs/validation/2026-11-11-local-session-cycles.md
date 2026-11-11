# P-05: LocalSession SOCKS, 100 циклов

База: `d0a281b4c23b45f65db1ddb74c6b14378b399794` плюс новый opt-in example
`crates/yonik-linux/examples/local_lifecycle_resource.rs`. Production API не изменён.

## Результат и масштаб

Изолированный компонентный прогон: **100/100 PASS**, попеременно 50 Xray и 50
sing-box stock из действующих desktop pins. Каждый цикл вызывает настоящий
`import_bytes → compile → BinarySnapshot::load → CheckedConfig::validate →
LocalSession::start`, передаёт уникальный 16-byte marker через локальный inbound
ядра в свой synthetic SOCKS upstream и выполняет явный `stop().await`.
Upstream проверяет literal destination tuple и marker. Он не открывает внешний
endpoint; DNS и системные маршруты для трафика не используются.

После Stop подтверждены Disconnected, отсутствие engine_pid и исчезновение
конкретного PID/starttime своего core. Version/config-check также вызывают
настоящие pinned binaries; подмена ими lifecycle-проверки не выполнялась.
Health `LocalSession::check` и пользовательский Connected не проверялись:
после start сессия могла оставаться Checking до Stop.

| Метрика после каждого Stop | Min | Max |
|---|---:|---:|
| FD стенда | 10 | 11 |
| Threads стенда | 1 | 1 |
| PSS, bytes | 3674112 | 3743744 |
| Private dirty, bytes | 483328 | 552960 |

Максимум start→marker→Stop — 80 ms. Сюда не включены
предварительные hash/version/config-check. Debug Rust 1.85.1, current-thread
runtime. Это не p95 GUI и не измерение активного native-runtime peak/group PSS.

Oracle объявлен до запуска: 100 traffic/reap outcomes, после 10 циклов прогрева
FD≤baseline+2, threads≤baseline+1, PSS/private dirty рост≤16MiB. Негативные
контроли неполного прогона, растущих FD/threads и retained memory прошли RED→GREEN.
Такая граница не доказывает отсутствие всех малых утечек или устойчивость 8 часов.

## Первый FAIL сохранён

Первый запуск в host network namespace завершился после 43 успешных циклов:
следующий sing-box вернул EngineExited до readiness. Причина **не установлена**.
Контроль с удерживаемым UDP socket не подтвердил гипотезу UDP-конфликта: TCP
listener запускался. Повтор 100 циклов с неизменным окружением не выполнялся.

Для следующего различимого прогона стенд переведён в отдельный user/net namespace
с единственным loopback; порт удерживается до завершения config-check, добавлен
bounded структурированный startup failure log из EventLog. Host network не менялся.
Успех в этом окружении не объясняет первый FAIL и не закрывает сетевые transitions.

## Provenance и проверки

Перед actual run сохранены source/core/binary hashes; exact executable SHA-256:
`ae05026534ec7bd7506b866de6c330e36b9a33ee2195d9747ac86f2f038b8160`. Snapshot источника в `isolated-attempt-2/source.rs`,
его SHA-256 `aaeda74b9c38820683fb77363e745eefa1d155177f36efc0f8e8bceaf6cd0345`. После этого только wildcard import внутри
`cfg(test)` заменён явными imports, runtime bodies не менялись; отдельная запись
`test-imports-only.json` сохраняет оба source hashes.

Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/p05-local-cycles/`.
Первый binary/source/log сохранены в `attempt-1/`; actual manifest/100 JSONL rows/
namespace witness/summary — в `isolated-attempt-2/`. Raw failures не исправлялись.
Пять unit/async oracle/peer controls PASS. Package `cargo test -p yonik-linux`: 489 passed, 32 ignored, 22 suites; ignored gates этим прогоном не принимаются. Pinned Clippy 1.85.1 driver `--all-targets -D warnings` и fmt --check — PASS. Никаких runtime bodies после actual run не менялось; последний delta только test imports.

Полный P-05/G-POWER остаётся открытым: controller/GUI group memory, actual health,
system TUN/network resources, новые transitions, оба редактора, physical idle/energy
этим компонентным стендом не подтверждены.
