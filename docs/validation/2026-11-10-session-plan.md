# Профиль и сеть системной сессии — 10 ноября 2026

База: PR #39 влит как `6355150`, PR #40 — как `49debdd`; на точных heads
`acc2368`/`362fbae` все пять jobs обоих workflows прошли. PR #40 перед слиянием
перенесён на main. Независимые reviews и исправления описаны в предыдущих evidence.

Новый срез: [контракт](../specs/helper-session-network.md), ветка
`feat/system-session-plan`. На пользовательском host сеть/units/подписки не менялись.

## Что проверено

- ServerIntent roundtrip шести протоколов, Reality/Vision, WS/Host с портом,
  gRPC/authority и XHTTP/ALPN; неизвестные/повторные поля, UUID, host, credentials,
  размеры/глубина/ALPN и редактированные Debug/errors. Публичный Deserialize
  отсутствует, что проверено compile-fail doc-test.
- Синтетические topology cases: точное исключение своих ресурсов, foreign
  addresses/routes/rules/nexthops/child links, duplicates, недостающие записи,
  pending reconciliation, cleanup и неподдержанные Session resources.
- Настоящий dual-stack TUN в собственных user/net/mount/PID namespaces:
  проекция после добавления addresses/routes/RPDB совпала с исходной topology.
- Второй actual gate использует SessionNetworkPlan целиком: configure → capture →
  verify_external → cleanup. Kernel snapshot возвращается к baseline, Session journal
  пуст. Compiler получает имя/mark/адреса из того же плана.
- Новые cases `tun-projection` и `session-network` запускаются штатным
  `tools/managed_network_smoke.py` и добавлены в обязательный Rust CI job.
  Runner предварительно проверяет `--list --exact`: ноль тестов не считается успехом.

Проверки использовали только синтетические endpoints/credentials. Игнорируемые
gates имеют обязательный guard отличия netns от host и запускаются отдельно;
обычный workspace test не выдаётся за actual gate.

## Проверки

Финальный workspace: **506 Rust-тестов**, 0 failed, 24 ignored, 48 suites.
Workspace clippy all-targets с `-D warnings`, fmt: PASS. **118 Python-тестов**,
Ruff 0.17.4 check/format всех 42 tools: PASS. Отдельно оба новых namespace cases
через штатный runner: PASS. Другие ignored gates не посчитаны выполненными.

## Исправления и ревью

Независимое review ServerIntent выявило обход bounded входа через Deserialize,
утечку неправильно типизированного секрета в serde-ошибке и отказ для допустимого
HTTP Host с портом. Вход оставлен только через bounded parse_json, ошибки скрыты,
HTTP authority проверяется отдельно от DNS hostname. Regression Host сначала
упала, затем прошла; узкий повторный review без findings.

Review проекции не нашло новых дефектов и подтвердило указанную в контракте
границу per-link metadata. Review сетевого плана и узкий delta review выполнены.
Root дополнительно устранил две ошибки: stale Confirmed без свежей reconciliation
и коллизию session TUN ID 1 с public policy. Обе регрессии воспроизведены до
исправления; после исправления прошли. Последовательность семи ID используется
во всех configure/capture/cleanup путях.

Логи вне TTL: `local/validation/session-host/`: `intent-review-{red,green}.log`,
`network-plan-{red,green,public-id-red}.log`, `final-linux-tests.log`,
`final-clippy.log`, `final-tun-projection.log`, `final-session-network.log`,
`python-tests.log`, `ruff-{format,check}.log` и workspace logs.

## Остаток

Нужны SystemSessionHost с неизменяемым artifact и свежей авторизацией,
объединённое владение journal с public protection, привязка собственного link,
worker+nft orchestration, readiness, Stop/crash/recovery, IPC/client/controller/GUI,
DNS/Fake-IP и смена сети. Этот срез не включает system_tun capability и не закрывает
P09/P12/P13 целиком. Общий план остаётся **3/26 (11,5%)**, открыты 23 пункта.
