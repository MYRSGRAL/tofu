# N5b: закрытая receiver-граница transport

База: `7e909c754ea2798a2c5615702224391b19e82ee1`.

Private move-only `VerifiedTransport` владеет exact `Binding` и двумя
полученными FD. Единственный receiver-конструктор сначала сверяет existing
YNKROLE1 с challenge/helper, затем вызывает existing receive_roles:
per-frame SCM credentials, exact frame, channel peer, live pidfd/starttime,
deadline и существующий ACK. Новый wire/roundtrip не добавлен.

RunHandoff хранит permit и transport; exec потребляет оба по значению.
ProductionPermit::exec_run больше не принимает публичные Roles и Binding.
Нет Clone, bool admission, env-конструктора или нового issuer.
Перед exec сохранена повторная проверка live pidfd/starttime.

ProductionPermit::mint по-прежнему безусловно отказывает, причём production
receive_run_handoff вызывает его до чтения ролей. Legacy Run, Backend,
native_session, anchor и authorization API не изменены. Полный N5 OPEN:
transport observation не является kernel admission; producer/factory не подключены.

## Проверки

Rust 1.85.1, jobs=1, общий with-rust-slot, отдельный locked transient target.

- `cargo +1.85.1 test -p yonik-linux --lib worker::role_handoff`: 9 passed.
- `cargo +1.85.1 clippy -p yonik-linux --all-targets -- -D warnings`: PASS.
- `cargo +1.85.1 fmt --all -- --check`, `git diff --check`: PASS.
- Новый настоящий roundtrip запускается отдельным test-child через собственный
  `unshare -Ur`; positive проходит без mint/exec. Негативы: чужой challenge
  (replay binding), reserved byte, starttime, неподходящий pidfd, deadline,
  helper PID, неподходящий channel. После каждого случая FD count = baseline.
- Existing guards проверяют все 192 байта frame, SCM wrong peer/count и cleanup.
- Mutation RED: удаление сравнений record с challenge в bound_record приводит
  к падению verified_transport_userns_roundtrip (case 1); оригинал восстановлен,
  повторный узкий прогон GREEN.
- Дополнительная mutation RED: снятие финального live_server в receive_roles
  также ломает новый roundtrip guard; исходник восстановлен.
- Compile-time ownership обеспечен private fields, отсутствием Clone/Copy,
  private exec_run и consuming self/transport. Полный issuer, runtime campaign,
  foreign UID и production exec не проверялись и не разрешались этим leaf.

Локальные исходник, test binary и полные логи сохраняются вне TTL:
`/home/myrs/projects/yonik/local/orchestration/evidence/n5b-receiver/`.

Roundtrip моделирует локальный transport в одном test-child; полноценный
production issuer, отдельный service process и health этим не доказаны.
