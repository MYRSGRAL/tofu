# Интеграция проверенных runtime leaves

База: `128a070884e82bf21b5c22a74ba908dadd7daf5a` (main, включая Clippy #162).
Проверенный исходный merge-кандидат: `816137878c1fb2ad74fa31483b28b535be2f8626`.

Обычные merge и точные родители:

- `131562484c930712bb04b38fbf9dfb30dddfc90f`: база + helper #168 `5a5e3beea896ee31b5043c1fe194adb4c7e74a49`.
- `97e1f346ea3f9d67b1152cb9edfc5c2ff34e58eb`: предыдущий + single RPC #161 `7934f1fb639945d9b14646873c52bfe06a94f0ed`, включая последний testfix.
- `816137878c1fb2ad74fa31483b28b535be2f8626`: предыдущий + cold #165 `86374a72745e89703dad1279bca981b058a62eb3`.

Первые два remerge не требуют ручных изменений. Последний объединяет только
тестовые exports bound_terminal_controls/cold_roundtrip/known_roundtrip и
адаптирует cold fixture к существующему serve(cancel), без caller metadata.
Session сохраняет TunRole::Bound(Arc<BoundTun>), test-only Fixture и owner metadata,
единственный CLOEXEC duplicate, Broker-private ValidatedProjection, final guard,
sequence, TTL/expiry. Frozen native_wire не изменён. RunAuthorization, custody
revoke, v3 report и legacy handlers унаследованы без fallback. Новых authority,
unsafe, capabilities и зависимостей поверх исходных leaves нет.

## Проверки

Rust/Cargo 1.85.1, --locked, jobs=1 через общий with-rust-slot, внешний target
под root flock. fmt --all --check и strict clippy --no-deps для Linux/nsfs --lib PASS.
Linux lib: 503 PASS / 36 ignored; nsfs lib: 3 PASS / 1 ignored, последовательно.
Это scoped lib проверка, не workspace/all-targets и не production admission.

Controlled kernel harness: финально 5/5 PASS. Первый запуск под параллельной
нагрузкой дал IPv6 Timeout в cold wire; остальные 4 gates PASS. Единственный
повтор без нагрузки прошёл целиком, код/лимиты не менялись. Оба лога сохранены.
Queue CLOEXEC/error, Journal custody/actual ATTACH и Backend unknown-stop/revoke/
confirmed-stop cleanup: 3/3 PASS в отдельных user/net/mount namespaces.
Первый custody harness с map-current-user отверг /tmp как UnsafePath; исправлено
окружение (map-root-user и namespace-local tmpfs), production guard не менялся.
Унаследованные stable rustfmt/cargo-clippy пути исправлены явными MSRV binaries.

Новые 16 probes здесь не применялись и не запускались; существующие v3 проверки
дают только report evidence. Cross-UID/installed-systemd campaign не запускалась.
Это не full native admission и не proof production kernel issuer.

## Evidence

`/home/myrs/projects/yonik/local/orchestration/evidence/runtime-reviewed-integration/`
содержит source.patch/hash, полные parents, remerge.patch, compile/lib/clippy/fmt
логи, retained Linux/nsfs ELF и SHA256, cold run.json, namespace logs и cleanup.
Linux ELF SHA256: `772fe425f16dd2db4677c8b4eb19b0ee9a7429bd31b5d1dde9cbe772f4c7bd3c`.
Это binding данного merge-кандидата; inherited log seals не удостоверяют всю
новую merged build. Собственный cargo target очищен штатным cargo clean,
transient root удалён; исходные ветки/refs и чужие файлы сохранены.
