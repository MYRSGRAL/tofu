# P11: типизированные ошибки scheduler service и interval

База delta: `503aceae7e99f57efeaff3c1f61ff5e206420367`, PR128.

DesktopSchedule сохраняет SubscriptionError до legacy mapping при load/restore и
reload. Backend сохраняет companion при Failed event из dispatch/next_event,
reconcile и продлении persisted interval. Cancel/shutdown по-прежнему используют
типизированный SubscriptionSchedulerError; их cancellation/join и generation
invalidations не изменены. Внутреннее поглощение service events в shutdown не
меняется этим delta. ACK slots, ключи, request channel и revision fences прежние.

Закрытые Stage SchedulerService/SchedulerValidation и reasons соответствуют
реальным вариантам SubscriptionError и RepositoryError. Старый map_subscription
по-прежнему определяет legacy key. Точное соответствие RepositoryError сохранено;
Storage не превращается в конкретный I/O kind. Fetch/Import остаются без cause.
Invalid interval получает companion только из InvalidLimits существующего
set_interval validator. Params остаются {}. V2 exporter сверяет scheduler generation
и допускает только error/source. Pins, capabilities, ABI и зависимости прежние.

Actual test создаёт encrypted repository и persisted source, повреждает только
собственный encrypted payload через Python stdlib sqlite3 и вызывает настоящий
load_at/reload_id. Получается RepositoryError::Corrupt. Второй producer —
replacement с нулевым interval, отвергнутым настоящим validator. Snapshot проходит
production strict V2 exporter; stale generation, wrong channel и отсутствие cause
отклоняются. RED с отключённым service companion падает на None вместо Corrupt;
это контролируемый regression experiment, не запуск всей исходной базы 503.
Первоначальные неудачные fixtures close-handle/wrong-key сохранены отдельно.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/p11-service-context/`.
Source snapshot сохраняется перед final checks; source-before/after.json связывают
build inputs, matrix и harness с финальной проверкой. Cargo.lock используется
без нормализации, offline/locked. Фактические версии rustc/cargo/clippy, retained
GUI binary, strict exports, логи и SHA256 manifest сохранены рядом. Старым логам
или бинарникам binding к 503 задним числом не присваивается.

Clipboard proof — actual producer snapshot → production exporter → настоящий Qt
clipboard RU/EN с извлечённым production errorText. QML replay не является полным
GUI fault injection. Invalid generation/channel/no cause сохраняют clipboard.
Полный P11, health/helper VM gate, Android и физические G-UI/G-POWER не заявляются.
Фактическое исчерпание счётчиков и каждый отдельный service failure не воспроизводятся.

Проверено: свежие 62 desktop library tests (включая parent maintenance) PASS;
GUI build PASS; Python V1/V2/factory 16 PASS. Обе clipboard replay кампании
Corrupt/invalid interval: RU/EN, independent strict V2 и refusal clipboard PASS.
Ruff недоступен в текущей среде; harness compile и actual execution прошли.
Terminal GUI clippy MSRV 1.85.1 `--all-targets -- -D warnings` и fmt PASS.
После terminal capture source-before/source-after совпадают.
