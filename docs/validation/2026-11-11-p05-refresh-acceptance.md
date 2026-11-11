# P05: ограниченная матрица acceptance

База: `b9e5071784065c1fcb4813608d195421a146f80c`.

Контракт: `data-and-contracts.md` §§1–3, `validation-and-roadmap.md` §матрица
проверок и FR-06, `PLAN.md` P05. Пустой ответ не означает удаление профилей:
контракт запрещает автоматическую замену рабочего snapshot. Частичный импорт
показывает accepted/rejected и требует явного подтверждения.

| Requirement | Existing test / proof | Недостающий actual gate |
| --- | --- | --- |
| URL credentials rotation, стабильные source/profile IDs и favorite | subscription_refresh_contract::source_token_rotation_preserves_profile_id_and_favorite | сочетание rotation с full/partial/empty/corrupt/HTTP/304/size и scheduled |
| Серверные credentials меняют fingerprint | profile identity baseline (accepted) | DesktopHost manual/scheduled, изменение видимого profile ID |
| Empty/partial сохраняют рабочий snapshot до решения | subscriptions::partial_import_keeps_old_until_exact_report_is_approved, failed_http_and_empty_refresh_preserve_committed_snapshot | DesktopHost manual/scheduled матрица |
| HTTP/304/size | subscriptions::conditional_304_updates_validator_without_replacing_body; subscription_refresh_contract::failed_wire_responses_preserve_every_persisted_snapshot_field | сочетание с rotation и DesktopHost |
| Cancel/stale CAS | accepted PR95 и repository identity baseline | не повторять всю proof; комбинации с rotation остаются отдельным gate |
| Proxy failure без DIRECT | subscriptions::proxy_policy_never_falls_back_to_direct; subscription_proxy::proxy_failures_never_fall_back; accepted PR96 DNS | actual pinned cores + DesktopHost manual/scheduled после rotation остаются gate |
| Redirect/TLS | fetch/service bounded tests | actual own CA/private namespace, origin headers после rotation остаются gate |
| Ограничение ресурсов | импорт ограничивает тело/число записей | size header rejection не доказывает 10x dataset/resource bound; gate остаётся |

Новые файлы `crates/{yonik-linux,yonik-desktop}/tests/p05_acceptance.rs`
не меняют production. Матрицы не являются полной приёмкой P05. RED/GREEN для
production исправления не заявляется: конкретный production дефект ещё не доказан.
Секретные тела/запросы fixture не экспортируются; используются синтетические данные.

Добавлены actual gates:

- Service manual/scheduled: 14 сочетаний после rotation URL token; стабильные
  source/profile ID, favorite, revision/source generation, отсутствие старых
  conditional headers; scheduled full/partial требует approve.
- DesktopHost manual/scheduled: 16 сочетаний; rename сохраняет ID, изменение
  серверных credentials создаёт новый ID, partial показывает accepted/rejected,
  304 проверяется отдельным чтением encrypted repository с deadline до shutdown.
- Service/scheduler: ненастроенный Proxy после rotation не обращается к DIRECT
  listener и сохраняет offline snapshot. Это не pinned-core transport gate.
- 200/2000 профилей: вход <256 KiB, импорт <15 секунд, чтение явными страницами
  по 100, проверка полного множества уникальных ID. Это не измерение RSS.

Первый 10x прогон выявил неверное ожидание теста (default page limit 200,
а ожидалось <=100); тест исправлен на явный limit=100. Это не production RED.
Host gate использует существующий backend harness; 43 включённых теста
отфильтрованы, их повторное выполнение не заявляется.

Запуск: отдельные user/network/PID namespaces, только свой loopback,
DesktopHost под private dbus-run-session, ephemeral repository key. Rust 1.85.1,
locked, jobs=1, общий rust slot, flock корня transient, единый target.
 Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/p05-refresh`.

Усиление 304 oracle обнаружило гонку первоначального fixture: reload сразу
после записи ответа мог отменить refresh до commit. Oracle теперь ждёт revision
в repository до shutdown. Production-дефект по этому падению не заявляется.

Финальная проверка: Linux acceptance — 3 passed, DesktopHost matrix — 1 passed
(43 existing tests filtered), Rust 1.85.1 locked workspace clippy all-targets
`-D warnings` — exit 0, fmt check — exit 0, git diff check — exit 0.
Actual матрицы выполнялись за 2,92 и 1,62 секунды соответственно.
Namespace D-Bus сообщил warning об inotify каталога /root; private bus и Host
завершились штатно. Политика TLS/host networking не менялась.
Evidence и оба исполняемых тестовых артефакта сохранены вне TTL в указанном
каталоге. P05 остаётся частично выполненным; production fixes и RED/GREEN
для них отсутствуют, полная приёмка не заявляется.
