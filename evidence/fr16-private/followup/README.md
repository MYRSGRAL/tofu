# Продолжение private FR16 leaf PR204

Исходный head: 2ce8fa6b79f54f6d4c5a01fd596a3e2d96dc97b1. Старые 77/4 относятся только к нему. Новый focused ELF: artifacts/followup-tests (в родительском evidence-каталоге, вне TTL, не коммитится). Старый ELF сохранён без изменений. Новый фильтр выполняет только worker::output_transport: 15 passed, 0 ignored, без kernel-query tests, native producer, Go execution или Runtime16.

## Golden compatibility

YNKROLE1: статические exact192 bytes для существующего accepted Rust Binding/record fixture (nonce=[1;32], digest=[2;32], epoch=3, session=[4;16], generation=5, invocation=[6;16], PID=7, start=8), включая reserved zeros и exact roles. Это только pure serialization, не реальная identity/permit.

ABI260: ссылка — commit 260bc50eeb4f3a40ac97cd9a54ff0ea943a833ef, не размер сообщения. SHA256 в abi260.sha256 вычислены из git objects этого commit, до запуска тестов byte-exact сверены с текущими native_wire.rs, JSON vectors и frozen spec. Golden тест проверяет все три SHA, точное равенство существующих Rust/Go serialized JSON fixtures и вызывает существующие Rust Header::decode/selected_features для accepted vectors. Нового native vector или producer proof нет и не заявляется. Полный native roundtrip/production acceptance остаётся закрытым gap вне leaf; для статического ABI compatibility существующих vectors достаточно.

## Source-mutants

Fixture generator (--check) строит unmodified-source snapshot из канонических output_transport.rs/compatibility.rs. Единственные общие адаптации: удаление module docs/test attributes и видимость test helpers для wrapper; EventLog не копируется, используется настоящий crate API. Baseline запускает те же четыре целевых assertion paths и проходит. Каждая mutant fixture отличается от baseline ровно одной source-заменой; diff сохранён отдельно.

- PIPEFS: убрать guard → named FIFO проходит validation → конкретный assertion validate().is_err().
- roles: убрать проверку 96..100 → malformed roles/count → assertion offset 96.
- resources: заменить v2 resource input на legacy header (обход resource validation в приватном codec) → malformed resource принимается → assertion decode(&bad).is_err(). Это не мутация production decoder.
- lifetime: заменить исходную generation на 2 → поздние bytes/EOF g1 получают неправильную generation → assert_eq contexts показывает точные left/right.

Wrappers используют should_panic с конкретным текстом assertion; ошибки setup/timeout через expect имеют другой текст и не засчитываются. Это source mutations в изолированных fixture-копиях, не только malformed-input tests. Канонические guards не менялись; они восстановлены до freeze по построению. Source freeze включает все crate files (также .fixture/.bin), Cargo inputs и внешние ABI include inputs. Generator сверяется до сборки и повторно после неё. Копии намеренно ограничены cfg(test) модулем; ни production authority, ни новый EventLog classifier не создаются.

## Проверки и границы

Одна additional focused Rust 1.85.1 test compilation (-p yonik-linux --lib --no-run), один transient target под корневым flock/with-rust-slot, jobs=1, wrappers cleared. Все варианты скомпилированы вместе, без четырёх пересборок и без правок во время build. Старый retained ELF не содержит новых тестов (проверен --list).

Первый запуск с параллельными тестами: 14 passed, foundation failure_and_drop_close_owned_writers получил EAGAIN; лог parallel-tests-red.log. Гипотеза: конкурентный fork/exec в другом foundation-тесте кратко удерживает writer до exec. Причина отдельно не доказана; существующий тест не исправлялся в этом leaf. Один последовательный повтор того же ELF: 15 passed; новые source-mutants/baseline/goldens проходят в обоих прогонах. Это не замена реального RED мутантов: их assertion panic messages сохранены в tests.log.

Full FR16 production НЕ PASS. Compile-fail non-Clone/reconsume proof не добавлен. Go/native files, production wire/decoder/send/Apply/DBus/helper/exec wiring, Cargo deps и unsafe не менялись. Исходный root Cargo.lock восстанавливается после проверки; compiled lock соответствует прежнему сохранённому normalized lock.

Финальная проверка: default check, fmt, clippy --all-targets -- -D warnings и Ruff check/format прошли. Clippy сначала отклонил intentional `false ||` в двух source-mutants; добавлены только scoped allow(clippy::nonminimal_bool) для этих двух модулей. Сохранён точный source_mutants.rs.reference, с которым собран ELF, и test-build-source-after.log доказывает неизменность compiled inputs до lint-only правки. ELF НЕ пересобирался: финальные 15/0 — повтор сохранённого нового ELF с pre-allow исходниками; финальный source отличается лишь двумя lint attributes/comments в harness (плюс форматирование generator вне compiled inputs). final-source-before/after фиксируют уже эту редакцию при clippy и повторе ELF; не заявляется byte-exact build финального head. Это ограничение lineage явно сохраняется. Все исполняемые функции, fixtures и assertions между test-build freeze и финальным freeze byte-exact неизменны.
