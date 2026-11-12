# PR116: принятые PolicyApply и typed P11 из main

После проверенного локального checkpoint `b2f3159` выполнен ordinary merge
exact main `e46e14157b54267069c3c80c70f6a99f08d0f38f` с принятым PR128.
Единственный конфликт: maintenance admission. Сохранены `preset_job.is_some()`
в busy predicate и typed `maintenance_refusal` с envelope generation из main.
Command generation ACK, scheduler drain fences и typed scheduler/service contexts
сохранены. P17 CAS/ACK и PolicyConnect cancellation из checkpoint не заменялись.

Предыдущий checkpoint с main 3f73b6a прошёл tests desktop/linux/import/engine,
workspace и GUI all-targets clippy Rust 1.85.1 -D warnings, GUI build и fmt-check.
Его evidence: `/home/myrs/projects/yonik/local/validation/pr116-policy-integration/`.

Final evidence, source hashes, fresh GUI binary, binary hash и toolchain:
`/home/myrs/projects/yonik/local/validation/pr116-final-integration/`.
Совместимый transient target переиспользован; Cargo последовательно, jobs=1,
with-rust-slot и flock task-root. Source во время проверок не меняется.
Неизменённые Qt/native campaigns не повторяются. Android, физические G-UI/G-POWER
отложены; system_tun=false и mixed compiler Unsupported не пересматриваются.

Финальные проверки PASS: cargo test desktop/linux/import/engine --locked;
clippy 1.85.1 workspace all-targets и desktop gui,test-support all-targets,
-D warnings; GUI build; fmt --all --check; factory 2 теста, diagnostic Python
5 тестов и RU/EN gate. Typed manifest, maintenance shutdown/drain, P17 matching
unavailable ACK и PolicyApply/SourceChange cancellation tests проходят.
P17 fixture обновлена для новой сигнатуры reject_unavailable_command (cause,
generation); первый compile failure сохранён отдельно. Прежние Qt/native
campaigns не повторялись, fresh runtime acceptance сверх Rust tests не заявляется.
