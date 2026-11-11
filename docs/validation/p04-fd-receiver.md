# P04: граница canonical FD receiver

База: `3c0c4e6d062a540cec439cc7d031d1e0696fb3a7`, ordinary merge
PR90 `7fedcd07014b410ecb278b19be58c55d5f2d8775` и PR101
`8b7261ef81978b3dc473b913c0a5ccc92f512cc1`.

Receiver — независимый preflight, пока не подключён к privileged dispatcher.
Он не является Prepare, Apply, admission reservation или доказательством custody.
Existing six RPC и closed capabilities-v1 не изменены.

Предлагаемый additive transport требует согласования root: отдельный
PrepareBundleV1 с bounded typed request и ordered canonical source FDs.
Source digest отличается от native artifact digest в существующем manifest.
Helper должен закрыто проверить source records, перестроить native artifact
существующим encoder и сверить native manifest/config/policy/revision/bundle
identity до выдачи одноразового prepared handle. Caller paths не принимаются.

Receiver проверяет весь batch metadata до первого чтения: count <=16, format v1,
records <=1.5M, checked physical arithmetic <=64MiB, read-only regular memfd,
четыре обязательных seals, exact length и уникальность digest. Читает через pread,
не меняет общий file offset. SHA-256 и точный record count проверяются вместе с
закрытым canonical grammar trusted evaluator. JSON/native protobuf/compressed
payload, regex и неканонические CIDR отвергаются даже с корректным caller SHA.
Отмена проверяется перед каждым блоком <=8192 bytes и после окончания.

Physical preflight учитывает три копии source и claimed compiler work; это не
процессная reservation и не RSS promise. До production необходим отдельный
aggregate pool, учёт native output/index allocations и удержание до доказанного
worker+cgroup empty либо bounded quarantine. Drop/Stop/owner loss не evidence.

App AND / BlockedProcessAttribution и Domain+CIDR / Real provenance gates остаются.
150k membership component proof не является production Apply. Полный P04,
actual both-core worker dataflow и isolated system-bus VM gate этим receiver
не подтверждаются.

## Отдельные semantic и runtime identities

Новый `IntentBundle::digest` использует `yonik-intent-bundle-v1\0` и связывает
bounded typed ServerIntent serialization, policy identity, ordered typed bindings,
revision, engine/pin, закрытые compile requirements, ordered canonical descriptors
и expected native refs. Runtime ports и probe secrets не входят в intent.
Текущий slice requirements — SystemTUN / IpOnly без domain provenance;
requirements не считаются доказательством capability/provenance.

`runtime_bundle_digest` использует `yonik-runtime-bundle-v1\0` и связывает intent,
actual config digest, helper retained context digest и immutable ordered native
manifest. Helper обязан вычислять context digest из собственного retained
owner/epoch/plan/private context. Старый `yonik-bundle-v1` не переопределён.
Эти функции пока не подключены к prepared handle, dispatcher или client reply.

Original canonical proof хранится отдельным `c-<native digest>.enc`, AAD
`yonik-canonical-source-v1\0` + native descriptor + bounded source header.
Ни index dedup, ни native bytes не используются для восстановления original
source. Native association перед публикацией воспроизводится существующим
encoder и проверяется SHA/length/count. Current/previous state schema прежняя;
sidecars учитываются inventory и reservations, автоматического prune нет.

## Проверки и оставшаяся интеграция

Rust 1.85.1 / `--locked`, jobs=1, общий `with-rust-slot`, transient root BSD flock.
Evidence: `/home/myrs/projects/yonik/local/evidence/p04-fd-receiver/`.

Receiver regression suite: metadata/hash/order/count/seals/readonly/cancel/budget,
closed grammar negatives, duplicate preservation и exact native hash при rebuild
для обоих engines. Large-case: catalog 100k domains + 50k CIDR, encrypted original
proof, reopen/export и native rebuild из canonical FD. Это encoder/store/FD proof;
никакие native binaries и production Apply этим тестом не запускаются.

Для завершения остаются negotiated PrepareBundleV1 и closed reply DTO, client
owner fence/no-retry, helper fresh authorization и peer evidence, typed admission
и binding, runtime digest/plan/custody в одноразовом handle, private worker ABI,
worker-owned immutable artifacts и native asset paths, empty-evidence/quarantine
на Stop/Cancel/owner loss/recovery. После этого необходим isolated system-bus VM
с реальными обоими pinned cores. До этой проверки P04 остаётся незавершённым.

Свежая локальная проверка slice: 7 integration tests, 3 store regression tests,
`cargo +1.85.1 clippy --locked -p yonik-linux --all-targets -- -D warnings`,
`cargo +1.85.1 fmt --all -- --check` — PASS. Проверка catalog regression отдельно
также PASS. API canonical store ограничивает один source/native proof 16 MiB;
это дополнительное ограничение physical admission, frozen encoder 128 MiB и
512 MiB disk cap не повышены. 64 MiB pool не является доказательством RSS.

Изолированный процесс fixture (без Cargo/rustc): large-case обоих engines завершён
за 17.395 s, peak RSS 27 232 KiB по `getrusage(RUSAGE_CHILDREN)`; каждый export
возвращает два canonical FD. Это footprint конкретного encoder/store/rebuild
fixture, не worker/VM RSS и не обещание при 10x нагрузке.
