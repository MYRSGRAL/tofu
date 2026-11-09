# Настраиваемая HTTPS-проверка

Основание: FR-10 и networking §7. Default target требует ровно HTTP 204.
`ProbeTarget::with_expected_status` принимает только 200…299; `accepts` определяет
успех для конкретного target. `ProbeResult::passed` означает только стандартный 204.
LocalSession и CLI используют target predicate. CLI получил `--check-status`.

GUI задаёт HTTPS URL и ожидаемый статус до Connect либо восстанавливает default.
Actor проверяет значение до замены прежнего, хранит URL вне DTO и передаёт сессии
собственный immutable ProbeTarget. При активном ядре Apply/Reset отклоняются;
смена source/selected profile не меняет проверку активной сессии. Настройка на
текущий сеанс, без скрытого фонового запроса. GUI timeout — пять секунд.

URL скрыт по умолчанию, очищается после отправки; ни URL, ни query token не
возвращаются в state/tray/trace. Результат активной сессии показывает HTTP-код и
длительность либо TLS/timeout/transport. Отдельная причина неожиданного HTTP-кода
не смешивает mismatch с отказом TLS. Редиректы остаются отключёнными; системный
proxy и прямой fallback не используются.

## Доказательства

- MSRV Rust 1.85.1: workspace 149 Rust-тестов + 4 doctest прошли, два inherited
  environment-dependent теста ignored. Новый contract-тест проверяет default 204,
  mismatch 200, custom 200, 302/503, TLS/timeout/transport и некорректные статусы.
- Девять actor-тестов прошли. Новый тест проверяет custom target, отказ HTTP URL,
  credentials и статуса 302, сохранение прежнего значения и отсутствие URL/query
  token в каждом DTO. Сброс возвращает default 204.
- `tools/desktop_smoke.py --case health`: шесть release GUI сценариев на настоящих
  закреплённых Xray 26.10.31 и sing-box 1.14.4. Собственный VLESS peer передаёт
  HTTPS к собственному loopback TLS server с отдельным CA. Для каждого ядра:
  ожидаемый 204 → Connected; неожиданный 200 → Limited; ожидаемый 200 → Connected.
  Смена target при активной сессии отклоняется с busy, прежний статус сохраняется.
  Каждый сценарий заканчивается Stop → Disconnected → closed, PNG и bounded trace.
- Fixture подтверждает реальный входящий HTTPS-запрос. CA задаётся только через
  SSL_CERT_FILE тестового процесса; проверка TLS не отключается. PID namespace
  приложения и cleanup peer не оставили процессов. Сеть/маршруты/DNS хоста не менялись.
- Workspace/GUI clippy с `-D warnings` и fmt прошли; release GUI собран; Android default-feature compile-check прошёл.
  Vendor warning GCC16/Qt6.11 в QChar остаётся прежним и не выдаётся за чистый
  C++ warning baseline. CLI отдельно отклонил `--check-status 302` и нечисловое
  значение до чтения профиля/запуска ядра.
- QML lint завершился без предупреждений. Python: ruff 0.17.4 format/check и четыре
  unit-теста прошли. Собственные PNG проверены: Connected/HTTP 200 и
  Limited/неожиданный HTTP 200 с причиной, видимой в UI.

Evidence вне временной сборки: `local/validation/configurable-health/results.json`,
шесть PNG и whitelist JSONL рядом. Использованы только вымышленные credentials,
реальная пользовательская подписка этим этапом не загружалась.

## Открытые границы

Transport adapter пока не разделяет DNS/connect/tunnel; сетевые поколения,
история нескольких измерений и повторная проверка активной Connected-сессии
ещё не реализованы. FR-09 TCP — отдельный незавершённый путь. Настройки GUI не
персистентны. Живой Wayland/tray/keyboard/HiDPI остаётся открытым, так как сессия
пользователя заблокирована; offscreen/software smoke его не подменяет.

Этот стенд доказывает здоровый HTTPS через выбранный реальный outbound, но не
доступность внешних провайдеров и не непрерывную гарантию интернета. Полный FR-10,
G-UI/G-POWER, TUN, DNS/Fake-IP и редактор маршрутизации не объявляются закрытыми.
