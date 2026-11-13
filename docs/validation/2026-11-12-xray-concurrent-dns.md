# Xray: изоляция authority при concurrent DNS и cache

База PR #126: `6de65807d74611eab1b58d72f0578e177729cc5a`.
Upstream Xray: `a4a3dff5b20c2faae567b742c172c7b95e75c083`.

`TestNativeDNSConcurrentAuthority` вызывает настоящий `LookupIPWithContext`
двумя независимыми closed fixture contexts: разные nonce и canonical local port.
Один вопрос обслуживает controlled TCP/UDP DNS server. Барьер по native fetch
в goroutine stacks подтверждает два ожидающих caller до выпуска ответа.
Матрица: TCP/UDP × serial/parallel × Known, cancel, revoke, epoch, Unknown,
Unavailable, stale, wrong session nonce, wrong config binding × оба victim.
Всего 72 сценария. Проверяются точный IP `192.0.2.17`, отсутствие адресов
у запрещённого caller, сохранение результата соседа, число wire queries,
повторный native cache lookup и routing cache после изменения authority.
Known и обычные отказы используют один exchange. При revoke ведущего socket
разрешён второй exchange от соседнего caller; это явно проверяется счётчиком.

RED исходного кода: cancel/revoke ведущего caller возвращал пустой результат
разрешённому соседу. Negative control с отключённым AfterDNS guard:
Unknown caller получал `[192.0.2.17]`, точное assertion падало.

Исправление: singleflight DoChan с отдельным ожиданием каждого caller;
общий exchange сохраняет session authority и revoke, но не caller cancellation.
При ошибке общего exchange сосед может выполнить один собственный exchange.
Отмена внутреннего DNS socket закрывает socket, не отзывает весь flow.
Managed UDP не переиспользует dispatcher/socket другого flow; DNS context
сохраняет закрытую authority. Cache публикует результат после записи данных,
чтобы новый lookup после завершения await действительно видел cache.
Положительная authority в DNS cache не хранится: выдача остаётся под AfterDNS.

Проверки: scoped race (новая матрица, прежние native DNS/Freedom/revoke),
полный package yonikbroker и routing DNS, DNS parser/config unit tests,
vet, native build, 18 builder tests, Go fmt, Ruff, Rust fmt и
MSRV 1.85.1 clippy для yonik-linux/yonik-engine --all-targets -D warnings.
Rust-команды выполнены последовательно через общий slot-wrapper, jobs=1.
Test-only TTL ClosedDNSFixture увеличен до секунды для scheduling под race;
production TTL/pins/caps/ABI не менялись. Stale остаётся отдельным отрицательным
контролем с истёкшим proof.

Evidence вне TTL: `local/evidence/xray-concurrent/` в выделенном worktree.
`hashes.json` содержит хеши source patch, metadata, binary, toolchain и логов.
Это компонентная closed-fixture приёмка native API и routing cache.
Фактический foreign UID/production ATTACH и production issuer W1 не приняты;
физические G-UI/G-POWER и Android не проверялись. Отдельного end-to-end
route/dial назначения после DNS эта матрица не добавляет; прежние Freedom
DNS и revoke callsite проверки входят в regression.

CI базы прочитан один раз: Rust job 130874222593 упал в
`attribution::broker::receiver_tests::rejected_rights_and_truncation_release_every_received_fd`,
`receiver_tests.rs:176`: `matches!(receive_frame(&rx), Err(Unavailable::Malformed))`.
Это другой failure, не inherited broker_private Known fixture. Полный лог
сохранён для root; Rust не редактировался, CI не ожидался.

Дополнительный upstream cache regression не принят: TestDOHNameServerWithCache
без closed context обращается к 1.1.1.1, получает «нет действующего broker proof»
и падает в common.Must. Это ограничение standalone unmanaged upstream test;
соответствующий лог сохранён, внешняя сеть/authority bypass не включались.
