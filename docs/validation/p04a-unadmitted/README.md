# P04A: unadmitted mixed data / cooperative cancellation

Root утвердил [минимальный контракт](root-decision.json) после R2 0d0908fe.
Исходный [actual RED](../p04a-mixed-red/README.md) выполнен **до** production edits:
exact Unsupported на обоих backend, exit42; wrong revision/port дают exit1.
Frozen RED ELF/source остаются в `local/p04a-mixed-red/` и имеют собственный binding.
Старые import/snapshot manifests и raw runs не изменены, baseline не повторялся.

## Типовая граница и исполненный GREEN

`UnadmittedCompiledData` хранит private bounded bytes/manifest/revision/identity.
Есть sensitive writer и readonly metadata; нет CompiledConfig getter, From/Into
CompiledResourceBundle или принятия в seal_bundle/Apply. Public VerifiedCatalog
не является authority. Policy/revision/identity/resources достаточно для **data**;
flow receipt, N5 и port provenance здесь не требуются. Ports структурно проверяются.
Старые compile/Names/IpOnly/process gates и сигнатуры сохранены. Нового activation API нет.
Xray AsIs не доказывает runtime DNS; system-tun condition не даёт права создать TUN.

`compile_bound_data_cancellable` проверяет structural/path/bounds и binding,
производит смешанный AND и сериализует не более 4 MiB. Checkpoints стоят в
validation/emission/serialization/final; false sticky в пределах вызова, наружу
возвращается Cancelled без artifact. Внешний host/store обязан повторно проверить
отмену перед publish/commit; этот callback не делает чужой commit атомарным.
Новый legacy cancellable sibling не добавлен: существующие фасады используют
общий emitter с always-proceed после прежних gates.

Actual synthetic `ResourceCatalog → encrypted store → sealed memfd` и отдельные
positive compile_bound_policy+seal_bundle компоненты проходят до mixed data.
[stdout](data.stdout) подтверждает обе реализации. Original ordered digest sequence
отвергает переставленные реальные payloads. Это **контроль fixture integrity**,
не обещание, что произвольно новая валидная перестановка bindings запрещена compiler API.
Wrong revision проверяется настоящим compiler entry. Path и фактическое превышение
4 MiB проверяются в engine suite; inline identity и остальные bounds при этом валидны.

[Actual pinned config-check](commands.json): Xray 26.10.31 и sing-box 1.14.4 exit0.
[Codec tests Xray](xray-codec.log) и [sing-box](sing-codec.log): actual source tag
decoders, resource formats/matchers и logical composition принимают match и
отклоняют wrong app/name/CIDR/inbound. Fixtures `.go.txt` копируются только в
восстановимый source checkout, production Go не изменялся.
Xray ProcessNameMatcher проверяется на точный emitted AbsPaths, затем OS lookup
заменяется test dependency; sing-box получает synthetic ProcessInfo. Эти проверки
**не** доказывают process-owner attribution, namespace authority или native Apply.
Go tests выполнялись offline с Go1.27.1, ELF config-check — существующими pinned ELF;
различие build toolchains не скрывается. Исходники обоих codecs/ELF hashes сохранены
в [binding](binding.json), исходные архивы вне TTL.

## Targeted measurements

Release Rust1.85.1, jobs1, with-rust-slot/max2 и shared flock собственного root.
По 1 warmup + 3 measured children на каждую комбинацию; каждый child строит реальные
100k distinct domains и 50k distinct /32 CIDR resources, делает четыре смены personal
rules, три cooperative cancel на раунд и recovery compile. Никакого раскрытия CIDR
адресов. 10/100/1000 — контрольные personal counts, 10000 — 10x stress от 1000.
Это **internal compile-only fixture**, а не обход UI 256 rules/64 group или PASS1000 UI.
Старые accepted 1000/10000-server и 20-subscription компоненты переиспользуются;
единого нового integrated baseline не запускалось.

| Backend | Rules | Compile median ms | Compile CV, 3 reps | Output bytes | Peak RSS KiB | Max cancel ACK ms | Recovery range KiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| xray | 10 | 0.041 | 5.58% | 3809 | 24032 | 0.008 | 0 |
| xray | 100 | 0.273 | 2.45% | 29729 | 23968 | 0.019 | 0 |
| xray | 1000 | 2.598 | 1.53% | 289829 | 23928 | 0.170 | 0 |
| xray | 10000 | 32.642 | 4.13% | 2899829 | 46384 | 3.341 | 20 |
| sing-box | 10 | 0.058 | 1.48% | 4472 | 26460 | 0.007 | 0 |
| sing-box | 100 | 0.413 | 5.96% | 34982 | 26408 | 0.038 | 0 |
| sing-box | 1000 | 4.561 | 22.74% | 340982 | 26436 | 0.555 | 20 |
| sing-box | 10000 | 58.179 | 3.93% | 3409982 | 104156 | 6.213 | 484 |

Compile median — медиана по трём child medians четырёх раундов. Raw per-round ns,
CPU/wall, sample variance и measured repetitions находятся в
[raw runs](performance/runs.json) и [summary](performance/summary.json).
Wall включает resource build, bind, assertions, recovery и startup; compile_ns
относится к actual cancellable compiler+serialization. Peak RSS через wait4 включает
bounded JSON oracle после compile timing, catalog/store и fixture strings;
это не чистый RSS компилятора и не PSS controller/helper.

Native top-level count строго rules+3, all nested nodes ≤5*rules+16; размеры
результата остаются bounded, ресурсы не размножаются на каждое правило. Это scoped
контроль разрастания и наблюдение роста, не доказательство асимптотики всех hotpaths.
Cancellation ACK измерен от фактического false callback до возврата Cancelled;
отдельный `ns` измеряет весь отменённый вызов. Проверяются начальный, validation
и final checkpoint; recovery вызывается после отмен. Unit sweep дополнительно
проверяет одноразовый false с возвратом callback в true и отсутствие продолжения.

После recovery в раундах 2–4 max-min RSS во всех measured children ≤8 MiB;
это достигнутый **инженерный scoped plateau**, не доказательство возврата всей
памяти ОС или отсутствия утечки за произвольный uptime. Actual max peak RSS и
разброс приведены выше; повышенный CV не маскируется новыми baseline runs.
Все восемь combinations реально COMPILED; runner различает REFUSED и не выдаёт
отказ за PASS. Для заведомо oversized fixture отдельно подтверждён output refusal.

## Воспроизведение и сохранность

На own transient root под shared flock и установленным with-rust-slot:

```text
CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=<own-root>/target cargo +1.85.1 build --release --locked --offline -p yonik-linux --example p04a_unadmitted_resource
python tools/perf_p04a_unadmitted.py <saved-release-ELF> <new-evidence-dir> <own-flocked-root>
cargo +1.85.1 test --locked --offline -p yonik-engine --all-targets
cargo +1.85.1 clippy --locked --offline -p yonik-engine -p yonik-linux --all-targets -- -D warnings
python -m unittest discover -s tools/tests -p 'test_perf*.py'
```

Explicit GREEN example: `cargo +1.85.1 build --locked --offline -p yonik-linux
--example p04a_unadmitted_data`, затем `<ELF> <new-output-directory>`.
Config-check команды и реальные exit записаны в commands.json. Codec команды:
`GOTOOLCHAIN=local GOPROXY=off GOSUMDB=off GOMAXPROCS=1 GOCACHE=<own-root>/go-cache
P04A_DATA=<xray-data> XRAY_LOCATION_ASSET=<xray-data> go -C <xray-v26.10.31-source>
test -count=1 -mod=readonly -p 1 ./app/router -run '^TestP04AActualCodecAND$' -v`;
для sing-box: source v1.14.4, P04A_DATA=<sing-box-data>, package ./route/rule.
Перед запуском соответствующий `.go.txt` копируется в package как p04a_codec_test.go.
Никаких production codec overlays или ABI/pin изменений.

Source SHA256 и ELF/source archive hashes сохранены до cleanup; нужные бинарники,
source tarballs, конфиги и полные raw logs (committed test log нормализует конечные пустые строки) находятся в `local/p04a-mixed-red/` этого
worktree вне TTL. Transient Cargo target очищается cargo clean после проверки
активных процессов, затем удаляется только собственный восстановимый root.

Данный leaf закрывает новый compile-only mixed/cancel/resource measurement gap.
**Full P04A не объявляется автоматически PASS** по сумме разных campaigns/исторических
bindings; root оценивает acceptance отдельно. **P04B/GAPP/TUN OPEN**, зависит от N5
и настоящего activation/native gate. Никаких новых native probes/runtime16/crossUID.
