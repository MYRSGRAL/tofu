# P09: сверка producer coverage и недостающий контракт

Проверенный исходный срез: `8ea826a` (accepted main), 14 ноября 2026.
Результат — **partial, API gap**, не новый runtime PASS. Production, P06/#198,
зависимости и прежние отчёты не изменены. Новых запусков host/core и сборок нет.
Исторические результаты ниже остаются привязаны к исходникам и бинарникам
своих отчётов; их наличие в ancestry не переносит runtime seal на `8ea826a`.

## Матрица исходного критерия

| Критерий | Существующее доказательство / producer | Что не доказано этим результатом |
| --- | --- | --- |
| Конкурирующие Connect | [#173](2026-11-14-local-socks-alternating-host.md): настоящий DesktopHost, pinned cores, busy/refusal при активной сессии; `local_socks_alternating.rs`, ветка `refuse`, посылает **set_engine**, не второй Connect | Одновременные Connect во время Validate/Apply. Отказ смены engine нельзя считать таким witness |
| Отмена Validate | [Host](2026-11-09-linux-engine-host.md): TERM во время version/config-check и timeout synthetic ELF; `host/tests.rs::cancellation_of_blocked_stdin_kills_and_reaps_owned_child` | Отмена реального pinned validation через обычный DesktopHost на подтверждённой границе. Synthetic ELF не actual pinned core |
| Отмена Apply | [System adapter](2026-11-11-system-session-client.md), `system_client.rs::adapter_gate_revision_and_unknown_cleanup`: cancelled Apply → Unknown; [GUI fixture](../../crates/yonik-desktop/tests/system_smoke.py), `applycancel` задерживает RPC reply | Fixture RPC не production helper/network effects и не LocalSession::start. Нельзя переносить Unknown на подтверждённую очистку |
| Позднее старое поколение | `prepared_plan.rs::cancelled_plan_cannot_damage_replacement_session_even_if_environment_changed`; `session.rs::stop_invalidates_callbacks_and_requires_confirmed_cleanup`; system adapter stale revision; `backend.rs::probe_producer_exports_typed_failure_and_rejects_stale_generation` | Reducer/producer assertions не actual старый validation/apply completion, выпущенный после нового Connect |
| Повторный Stop | `session.rs::cancel_preparation_and_duplicate_stop_do_not_revive_old_operation`; [Stop contention](2026-11-10-system-session-stop-contention.md): admitted Stop удерживает slot, второй Busy | Последовательные 100 connect/stop не повторный Stop одной операции; helper admission fixture не actual normal-host cleanup |
| Потеря host | `Cancellation` считает закрытие watch-источника отменой; `DesktopHost::drop` вызывает shutdown. [Старый host report](2026-11-09-linux-engine-host.md) явно оставляет host SIGKILL открытым | Закрытие control source при живом runtime, graceful shutdown и SIGKILL самого host — три разные причины. Drop при SIGKILL не выполняется |
| Подтверждённый cleanup | [#169](2026-11-13-local-socks-resource-100.md), [#173](2026-11-14-local-socks-alternating-host.md): Stop ACK + pidfd readiness + PID/starttime/reap + rebind; restored-positive/peer-negative отдельно | Namespace teardown в аварийном cleanup harness не product cleanup. Полная native/system матрица остаётся отдельной |

Пути тестов без полного пути в таблице: `crates/yonik-application/tests/`,
`crates/yonik-linux/tests/system_client.rs`, `crates/yonik-linux/src/host/tests.rs`
и `crates/yonik-desktop/src/backend.rs` соответственно.
Таблица описывает существующий код/зафиксированные reports, не новый прогон тестов.

Принятое исправление статуса #169/#173 уже находится в
[delta 302f8bf](2026-11-14-linux-delta-302f8bf.md): heads `88c398a`/`dac33e7`
приняты, старый incoming/cleanup-pending статус неактуален. Это не закрывает P09.
100 циклов #169/#173, P03 и старые reducer suites повторять для этой сверки не нужно.

## Точный API gap для детерминированного normal-host witness

В `backend.rs` последовательность реального local producer такова:
`BinarySnapshot::load` → `CheckedConfig::validate(...).await` →
`session.start(...).await`. Между validation и start нет управляемого внешним
tests/examples барьера. `LocalSession` сериализован через `&mut self`;
`start` внутри выполняет prepare/apply. Текущие test-support входы HostConfig
управляют cleanup fault, scheduler и disk, но не удерживают эти границы.
`Preparing` snapshot не подтверждает, что version/config-check уже выполняется
или что PreparedPlan получен, но Apply ещё не допущен.

Поэтому послать Stop после Preparing и назвать это cancel-Validate/Apply было бы
непроверяемой временной гонкой. Sleep и подмена pinned ELF synthetic процессом
также не дают нужного producer proof. Здесь такой тест не добавлен.

Минимальный контракт для согласования root: test-only наблюдение достигнутой
границы actual producer и одноразовое удержание/освобождение до эффекта Apply,
без возможности создавать CheckedConfig/PreparedPlan или подменять authority.
Отдельно нужна достижимая граница внутри validation для cancel-Validate.
Готовый witness обязан показать достигнутую границу, Stop/новое поколение,
позднее освобождение старого completion, отсутствие старого Connected/нового
listener и подтверждённый exit/reap. Контроль без отмены обязан реально пройти
Connect; негативный oracle должен отличать живой core/занятый порт от cleanup.
Это предложение контракта, **не разрешение изменять production API**.

## Отдельный crash gate

Host SIGKILL не требует нового API, но остаётся самостоятельным actual witness;
в этой сверке он не выполнялся. Из Rust Drop не следует ни PASS, ни наблюдавшийся
orphan. Для следующего узкого запуска supervisor должен пережить собственный
host, удержать private bus/XDG/namespace и наблюдать identity-bound pinned core
и listener после SIGKILL, до teardown изоляции. Живой owned core будет concrete
finding; затем supervisor обязан kill/reap только свои оставшиеся процессы.
Нужны exit/port negative canary и source/ELF binding вне TTL. Namespace exit
можно использовать для последней страховки стенда, но не как product verdict.
PDEATHSIG, новый syscall и user unit требуют отдельного root fix-contract.

Нативные helper/systemd/network доказательства не заменяются LocalSocks. Этот
документ не повышает P09, P12/P13, P05 или `system_tun` и не отменяет существующие
accepted source-bound proofs.
