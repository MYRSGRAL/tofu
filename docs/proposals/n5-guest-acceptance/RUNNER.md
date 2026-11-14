# N5: закрытый guest acceptance package

**NOT EXECUTED / EXECUTION CLOSED / MISSING FIXTURE / SOURCE BLOCKED.**
Этот пакет добавляет только proposal Python/JSON/docs. Импорт runner, AST и mock
unit tests не исполняют Desktop/helper/worker/engine и не читают runtime paths.
Ни USER software grant, ни review/CI PR216 не разрешают этот runtime campaign.
Никаких production profile registration, разрешений/UNIT bypass или fallback.

## Что именно подготовлено

`runner.py` — исполнимая реализация будущего guest-only контроллера: фиксированные
argv/env, потоковый bounded JSON protocol, verifier полей/полного lifecycle,
однократные Start/Stop, обязательный cleanup, own child pidfd TERM/KILL/reap.
Реальный driver не подменён mock-ом: его actual ELF/source/control receipts сейчас
**MISSING**. `test_runner.py` подставляет исключительно in-memory mock transport;
PASS тестов означает корректность runner/oracle, не actual N5 admission.

Source target: `3e3127d30f66ac4d41aa5d5e7ff643e08bef5fb8`. Exact git blobs
перечислены в `cases.json`. Это source pins, не trusted deployed ELF tuple.
Старые FR16 artifacts/test ELF/control bd5 source не считаются новой production
сборкой. Frozen fixed16 coverage identity отдельно от actual deployed worker
ELF/source/kernel identity. Контроль не изменяет frozen primitive/receipt.

Два конкретных blocker в этом source:

* `crates/yonik-desktop/src/system_session.rs`, `SystemSession::apply` вызывает
  `SystemClient::apply_output`. `helper/session_host.rs::start_inner` при
  `artifact.attribution_required` отвергает `output.is_some()` как Unsupported.
  Прямой Apply вместо default Desktop path не исправляет этот acceptance case.
* `packaging/systemd/yonik-helper.service`: `User=root`, CAP_NET_ADMIN.
  `attribution/broker/mod.rs::Broker::build` требует actual uid/euid == app owner
  UID и нулевые CapInh/CapPrm/CapEff/CapAmb. Thread в helper сохраняет credentials.
  Нельзя объявить root/helper thread independently admitted app-UID broker.

Они записаны в `authorization.json.source_blockers`; execution gate отвергает
непустой список. Для future source correction нужны отдельные product changes,
review, обновление source/blob/ELF pins и новый sealing. Пакет их не делает.
Также MISSING настоящий scoped observer: текущие private DATA4/issuer/roles не
становятся внешне наблюдаемыми от наличия JSON field или reporter digest.
Нужен source-bound, независимо reviewed guest driver/observer с provenance
реальных событий; synthetic event emitter не удовлетворяет fixture prerequisite.

## Fixtures и отдельное разрешение

Единственный umbrella prerequisite — **SEALED_GUEST_FIXTURE_V1** от W3: complete
owned guest/PID1/system bus/unit graph/admitted caller/runtime closure, immutable
actual-used artifacts и их source/build/provenance receipts. Base image сама по
себе этого не доказывает. Старые SSH failures/base не повторяются. Runner не
скачивает, не собирает, не устанавливает и не запускает VM; guest provisioning,
VM custody/serial transport/quarantine остаются отдельным W3 prerequisite.

Предлагаемое расширение sealed JSON (согласуется через root; до согласования
не считать finalized fixture schema):

```
schema = "SEALED_GUEST_FIXTURE_V1"
status = "SEALED"
source_commit = exact reviewed source
n5_acceptance = {
  caller_uid: nonzero u32,
  kernel_build_id: nonzero lowerhex40,
  artifacts: {desktop, helper, worker, source_archive, profile,
              kernel_notes, xray, sing-box, driver}: lowerhex64,
  controls: {each of 9 fixed case names}: reviewed control-receipt SHA256
}
```

Это extension, не замена полного W3 closure receipt. `kernel_build_id` здесь —
hex20 observer identity; installed sidecar schema1 использует **JSON array[20]**.
Sidecar должен проверять exact actual ELF/source, release/machine/build-id;
report/receipt hashes не являются самостоятельной production authority.
`artifacts.driver` должен связывать actual source-reviewed driver ELF; controls
ссылаются на immutable approved per-case guest artifacts/barriers. Не достаточно
caller bool `supported=true` или hash произвольного input. Пока нет реальных
артефактов, соответствующие поля — MISSING, не null/zero/template trusted pins.

Отдельный USER runtime grant обязан быть exact sealed artifact. Будущий grant:
`schema=N5_USER_GRANT_V1`, `explicit_user_runtime_grant=true`, source_commit,
fixture_sha256, runner_sha256 и exact 18 case IDs в deterministic order.
`authorization.json` после нового явного grant и review должен иметь
`execution=AUTHORIZED`, пустые source_blockers и actual runner/fixture/grant SHA.
Runner сравнивает actual bytes всех трёх с pins; CLI/env не включают authority.
SHA самого `authorization.json` должен быть зафиксирован outer W3 launch receipt.
Не включать authorization в hash самого себя: этот файл — separate approval input.
Исполнение без reviewed source correction и exact fixture/grant НЕ РАЗРЕШЕНО.

Fixed installed inputs будущего guest:

* `/usr/lib/yonik-acceptance/runner.py` и colocated `authorization.json`;
* `/usr/lib/yonik-acceptance/sealed-guest-fixture-v1.json`;
* `/usr/lib/yonik-acceptance/n5-user-grant.json`;
* `/usr/lib/yonik-acceptance/n5-guest-driver` — immutable executable, actual SHA
  из sealed fixture; driver и ancestors root-owned, без write/symlink substitution.

Runner будет работать **внутри** sealed owned guest от `caller_uid`; outer owner
обязан отдельно доказать VM identity/no host mounts/no outbound/closure/custody.
Host launch не является допустимым режимом. Запись произвольного fixture JSON
не заменяет эти доказательства. До PID1/caller/graph sealing выполнение запрещено.

## Точные future argv/env и protocol

Для каждого из 18 фиксированных сочетаний `cases.json.future_argv` содержит:

```
/usr/bin/python3 /usr/lib/yonik-acceptance/runner.py --execute --core xray --case positive
```

Допустимые core: xray, sing-box. Case — только enum из следующего раздела.
Каждое invocation выполняет ровно один case в одном owned guest campaign;
после UNKNOWN нет следующего Apply. Не добавлять shell commands, --command,
--fd, arbitrary executable/config/path или environment passthrough.
Только после future explicit grant; **команды выше сейчас не выполнялись**.

Driver argv всегда:
`[/usr/lib/yonik-acceptance/n5-guest-driver, --receipt-protocol=n5-v1]`, cwd=/.
Environment очищено и состоит ровно из:
LANG=C.UTF-8, LC_ALL=C.UTF-8, PATH=/usr/bin,
XDG_RUNTIME_DIR=/run/user/{sealed caller_uid}, WAYLAND_DISPLAY=wayland-0,
DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{sealed caller_uid}/bus.
System bus address не переопределяется; YONIK/test-support/native overrides нет.
Совместимость именно этой Wayland/session-bus topology доказывает W3 fixture.
Driver управляет реальным default Desktop/system-helper path через заранее
reviewed UI adapter и наблюдает actual canonical system units. Он не заменяет
PID1/private bus, не вызывает leaf worker напрямую, не mint-ит permits и не
регистрирует profile из CLI bytes. Source incompatibility → BLOCKED, не fallback.

Одна JSON request line: schema=1, strictly incremented seq, core, case, action.
Action — только start/await/revoke/cancel/close_desktop_owner/stop/cleanup из
фиксированной программы case. Один response на request, без unsolicited frames.
Driver никогда не принимает config/FD/PID/argv/secret от runner. Start единственный;
await наблюдает следующий actual event без повторного Apply/role-send.

Response имеет ровно schema/seq/core/case/stage/facts/binding/coverage/artifacts/
kernel_build_id. Binding включает canonical worker/server PID+start, invocation,
generation/epoch/app UID, **хеши**, а не raw nonce/session, queue/topology/digest.
Один binding неизменен до closed, включая exec с тем же PID/start/invocation.
Facts — строго integer0/1 (не bool): apply_sends/role_sends/admitted/ack/health/
revoked/retained/empty/reaped/closed. Артефакты точно совпадают с sealed inputs.
Coverage: independently observed DATA4 report/receipt hashes, bounded B/C,
C=B+1 без overflow и exact old3 [1,1,38] + fixed16 all1. Нельзя выводить факт
issuer/owning receipt только из этих чисел: это full observed lifecycle input
от отдельно reviewed real observer, который ещё MISSING.

## Case oracles

На каждом core фиксированы:

1. positive: profile → DATA4 → issuer → roles → ATTACH ACK → actual health.
   Accepted/EOF не readiness; нужны actual ACK, canonical live owner и health.
2. missing-profile: EvidenceUnavailable до issuer/roles/ACK/health.
3. tampered-profile: real immutable-contract mismatch, без admission.
4. tampered-report: actual bound worker exchange, bad report rejected до issuer.
5. revoke после roles: revoke немедленно запрещает использование; runtime custody
   удерживает FD/queue/invocation до independent confirmed stop.
6. cancel-before-handoff: barrier после issuer, до единственного role-send.
7. cancel-after-handoff: ровно один role-send; late ACK не возвращает readiness.
8. host-loss: исчезновение actual admitted Desktop owner после health.
9. unknown-stop: Stop unknown, no readiness, no early FD/owner release. Нельзя
   считать unit EOF/ACK/manager response доказательством empty/reap.

Каждому negative control нужен отдельно sealed artifact/control receipt. Нельзя
повреждать host/system fixture или создавать production test hook ради кейса.
Если source/adapter не поддерживает case — MISSING/BLOCKED; не synthetic PASS.
После каждого case Stop отправляется максимум один раз, затем только cleanup.
Closed требует независимые cgroup-empty + child reap + drain/transfer completion;
остановка driver или stdout EOF этого не доказывают. Неизвестный cleanup:
QUARANTINED, запрет PASS/new Apply; outer W3 owner сохраняет гостя и custody,
путь/size/reason локального quarantine receipt без экспорта raw secrets.

## Bounds и evidence

Максимум4096 bytes/frame, 64KiB input JSON, 1MiB driver protocol за invocation;
никакого raw stdout/stderr spool. Case60s + cleanup15s внутри campaign900s.
IPC read/write используют единый absolute monotonic deadline, без retry/reset.
Own driver получает pidfd сразу после spawn; TERM5s → KILL5s → wait/reap.
Driver нельзя заменить произвольным executable, а worker PID из receipt никогда
не становится kill target runner. Driver cleanup не заменяет guest cgroup stop.

Driver stderr discarded; raw protocol при ошибке не экспортируется. Публичный
результат — только case/core/status/fixed reason; сырые native/boot/config/nonce
логи не сохраняются/не отправляются. Numeric receipts хранятся только в bounded
memory для проверки; owner должен сохранить approved sanitized receipt manifest
вне TTL до guest cleanup. Приватный local evidence требует отдельного reviewed
outer fixture collector; сейчас actual runtime evidence отсутствует.

Разрешённые здесь проверки: AST syntax, `unittest` только pure mocks,
`ruff format/check`. Не запускать runner --execute, driver, Desktop, systemctl,
D-Bus/native/probes, worker/engine, compiler/build/install/VM/SSH/download/query.
Проверки runner не являются PASS его future production campaign.
