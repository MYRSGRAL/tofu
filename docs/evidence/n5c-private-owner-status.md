# N5c: закрытый producer и exact-owner transfer

База: `36cc6a9737e009652e6ca9af51c7f37ad22757c9`.
Сохранённый checkpoint: `512fc18161dd8bd793039f1fff7d4f27d93e6a6a`.
Контракт: frozen `7faab39f829ac6e10f10c0630fc1b4bbaf59d036`, с разрешённым
private расширением RunAuthorization/AuthorizedRun для exact owner/payload.

## Реализация

Helper-owned session16 закрывается при entropy failure/zero. Reservation использует
checked generation, failed compile не возвращает номер, replay tracking не запрещает
старые ещё не consumed artifacts. Legacy artifact сохраняет stamp=None.

Move-only network record удерживает exact Arc artifact и canonical queue. Существующие
проверки topology/route/journal policy выделены в общий метод с legacy verification;
producer дополнительно проверяет binding очереди и неизменность snapshot. Синхронный
commit повторяет final binding/peer UID checks и создаёт pending authority без await.

RunAuthorization::validate_owner требует живую revocation, тот же Arc RunRevocation
и session/generation/network_epoch. AuthorizedRun::validate_owner выполняет прежнюю
полную ticket/binding/custody проверку, затем owner link и exact payload Arc.
Digest equality не заменяет Arc identity. Expected payload берётся только из retained
SessionArtifact.payload. Внутренние token/FD/поля наружу не выдаются.

Transfer сначала повторно проверяет network context и exact AuthorizedRun, затем без
await перемещает весь pending lease и целый AuthorizedRun в active slot. Ошибки
отзывают authority, оставляя recovery ownership в pending. Unknown stop отзывает
pending/active, но сохраняет queue/artifact/run и Busy. Release происходит только
после подтверждённого recover. Drop и cancellation guards отзывают authority.

Фабрика, stamping compile callers, legacy start_authorized consumer, permit/report16,
wire/API и production runtime16 не активированы. Private dead_code ограничен закрытой
producer boundary, а cfg(test) fixtures не существуют в production.

## Unit proof boundary

Полные логи, runner и source/log hashes находятся в
`local/evidence/n5c-owner-completion/` вне TTL. Toolchain 1.85.1, jobs=1,
общий with-rust-slot. Проверяются:

- generation burn, replay, foreign session, zero/overflow и epoch overflow;
- exact record сохраняет artifact A после удаления исходной ссылки; одинаковый B
  не подменяет retained A; второй artifact вообще не принимается commit API;
- canonical queue Arc mismatch/absence, epoch/stamp change, coverage loss/exhaustion,
  journal ownership loss отвергаются общим production final-binding методом;
- одинаковые числовые owner fields при другом revocation Arc и изменённые typed fields;
- equal-bytes/equal-digest payload с другим Arc;
- transfer целого реального AuthorizedRun с guard/custody без early Drop;
- cancellation до Active, late revoked result, unknown stop retention и confirmed release.

Mutation RED требует assertion failure, не compile failure: удаление owner Arc check,
payload Arc check, canonical queue finalbinding, revoke и удержания run.

Итог проверки: 9 native + 1 authorization + 32 supervisor + 22 session теста
GREEN; 2 существующих session-теста ignored. Все пять mutation — assertion RED.
Rustfmt и `cargo clippy -p yonik-linux --all-targets -- -D warnings` GREEN.

## Точный внешний gate: positive canonical network/runtime admission

Positive fixture проверяет **настоящий общий unit final-binding boundary**, используя
retained unit artifact/queue и явно заданный snapshot. Она не является положительным
результатом verify_native_network и не подделывает kernel queue/network verification.
Production record constructor по-прежнему требует canonical BoundTun, подтверждённый
journal/network policy и полностью наблюдаемую сеть после effects. Получить такой
positive record в обычном unit процессе без отдельного runtime/kernel fixture нельзя.

AuthorizedRun fixture удерживает настоящий RunAuthorization, RunRevocationGuard и
NamespaceObservation test custody, но /dev/null observation не проходит canonical
PID1/cgroup identity. Её положительные transfer tests используют отдельный cfg(test)
link boundary. Production validate_owner не обходит identity и отвергает эту fixture.
Это проверено отдельным отрицательным тестом. Test custody не доказывает namespace
admission или непрерывное runtime enforcement.

Следующий внешний gate — отдельно разрешённая canonical queue/network + fixed worker
fixture, вызывающая реальные verify_native_network и AuthorizedRun::validate_owner.
Ни generic FD constructor, ни proof bool, ни factory/caller для обхода gate не добавлены.
Full N5/runtime enforcement OPEN; unit/link acceptance не следует трактовать как
положительный canonical runtime admission.
