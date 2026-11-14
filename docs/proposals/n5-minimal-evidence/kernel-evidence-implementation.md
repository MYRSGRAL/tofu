# Приватный DATA v4

`helper::kernel_evidence` не имеет production callers. Единственный результат
consume — `ConsumedRecord`, без преобразований в permit, Session или CompiledConfig.
Вектор хранится в source170 order: old3 unshare/setns/clone3, затем interleaved
unshare/legacy clone для known8. Syscall не исполняется.

Фиксированный frame 512, BE, reserved zero, nonzero identity integers, seccomp=2,
19 errno 1..4095. Неизвестные/короткие frames отвергаются. Все binding значения
берутся из private `Expected`; тестовая генерация wire из Expected — UNIT DATA,
не transport producer. Profile digest сравнивается только как DATA binding.
Receipt optional reference, включая ненулевой hash, не даёт authority.
`supported_profile()` безусловно возвращает EvidenceUnavailable: отсутствует
reviewed sufficient predicate и authenticated installation record.

`RunAuthorization::validate_owner` проверяет точный revocation object. Mutex
даёт один DATA consume; failure/drop отзывает guard; success переносит guard
в DATA record, drop пустого pending не отзывает его преждевременно. Owner revoke
остаётся действующим после переноса. Deadline локальный monotonic, не wire TTL.
Ни одного await здесь нет. Отзыв проверяется до framing и после parsing.

Ограничения: `Expected` не имеет production конструктора. Подключение canonical
owner/network/invocation stale checks, transport SCM/TRUNC/FD/pidfd custody,
profile provenance/actual-used observations и receipt producer не реализовано.
Реальные external случаи не считаются PASS. Карта всех 61 design cases находится
в kernel-evidence-coverage.json. DATA lifetime аналог не доказывает actual
cancelled blocking transport или health integration. Profile означает только несовпадение ожидаемого digest; provenance не проверено.
IncompleteApplication не синтезируется без authenticated inputs.
`permit_count=1` concurrent fixture означает один DATA record; kernel permit=0.

Для будущего trusted constructor может потребоваться read-only мост:
`RunAuthorization::network_epoch(&self) -> Result<u64, RunAuthorizationError>`.
Сейчас он не добавлен: integration owner/context вне этого leaf. Expected epoch
проверяется против wire, а token проверяется против реального owner; это ещё
не интеграция canonical pending context. Existing v3/ABI/factory не менялись.

Проверки: Rust 1.85.1, wrappers пусты, jobs=1, общий with-rust-slot.
Только `cargo test -p yonik-linux --lib helper::kernel_evidence::tests`:
6 tests; прочие 576 отфильтрованы. Clippy `-p yonik-linux --all-targets -- -D warnings`
компилирует targets, но не запускает syscall probes. Mutation RED проверяет
расширение errno upper bound, отключение nonce binding и ранний drop guard;
после восстановления выполняется тот же адресный GREEN. Evidence вне TTL:
`/home/myrs/projects/yonik/local/orchestration/evidence/n5-kernel-evidence/`.
