# P09: разрешённый fixed parent-death adapter и actual GREEN

Статус: review-ready, узкий LocalSocks fix; **не полный P09/native PASS**.
Primary grant: `local/orchestration/linux-authority-approvals.json:p09_local_parent_death`,
14 ноября 2026. Exact reviewed patch SHA256
`432646034693b379f08e99557096c4be97aee8a00437d8b6fcee027d06700d5d`
применён без изменений (включая после rustfmt), новый unsafe только fixed installer.
Предыдущие proposal тексты описывают стадию до grant; алгоритм не расширен.

## База, scope и источник

Обычный merge accepted main `b25ed1184d917a4ba9465b000b4bec080cfcaa3e` дал
`d2db073a313d4f3abf939acb86f3ab668880e87d`. Все четыре reviewed before-path
совпали с binding до apply; after-path hashes совпадают с reviewed patch.
Добавлены только `rustix1.1.6/process`, nsfs fixed adapter/export и один вызов
из private local `BinarySnapshot::command`; worker/factory/N5 и backend shared
логика не изменены task delta. Чужие accepted изменения вошли только merge.
Cargo.lock и версии dependencies не менялись. Поставленные target guards и
условие normal getpid/getppid/no errno emulation сохраняются.

Два ранее принятых RED остаются неизменны:
[sing-box 59ece58](2026-11-14-p09-host-sigkill.md),
[Xray 58fd4bf](2026-11-14-p09-host-sigkill-xray.md).
Оба использовали source8ea/ELF
`27cc3f3169f79af4cb636387a08ff26d4f9fbb5e984a36028fca2f0b5b642cfe`
и показывали survives/listener. Старый runtime не повторялся. GREEN ниже собран
на integrated b25 + exact patch, поэтому это не притворный A/B на одной базе.

## Exact-build receipt до первого candidate execution

Evidence вне TTL:
`/home/myrs/projects/yonik/local/evidence/p09-parent-death-granted/`.
`freeze.json` содержит 9470 hashes workspace/dependency inputs, toolchain,
config/env и профиль. До freeze записаны Cargo metadata и все новые tests/tools.
Во время Rust build/clippy compiled closure не менялся; повторная сверка это
подтвердила. Отдельный интерпретируемый verifier после build исправлен для Cargo
формата, его revisions явно сохранены, а не выданы за первоначальный freeze.

Rust1.85.1, exact `x86_64-unknown-linux-gnu`, jobs1, внешний CARGO_TARGET_DIR,
BSD flock самого root и shared `with-rust-slot` (≤2 globally), пустые wrappers,
RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS. Известный Cargo config содержал только jobs и
rustc-wrapper, последний overridden пустым env; effective rustc path/hash проверен.

Собраны только:

```sh
cargo +1.85.1 build --locked --target x86_64-unknown-linux-gnu \
  -p yonik-desktop --example local_socks_alternating -j1 -vv --message-format=json
cargo +1.85.1 test --locked --target x86_64-unknown-linux-gnu \
  -p yonik-nsfs --test local_parent_death --no-run -j1 -vv --message-format=json
```

`tools/p09_build_receipt.py` сопоставил **actual** rustc argv с точным compiler,
JSON `build-script-executed`, original build-script `output`, compiler-artifacts
и цепочкой example → desktop → linux → nsfs → rustix, а также test → nsfs →
rustix. Для Cargo1.85 rmeta сопоставлена с rlib того же output/metadata suffix;
идентичные duplicate extern разрешены только если путь один и тот же.

Две действительно разные linked rustix instances:

- Workspace: `librustix-d79d0486352e235d`, features alloc/default/fs/net/process/std.
- Standalone nsfs test: `librustix-c3408093c8ef9eb0`, alloc/fs/process/std.

Обе имеют generated **linux_raw и linux_raw_dep**, без libc/miri/use-libc/
rustc-dep-of-std/rustix_use_libc/rustix_no_linux_raw. Проверены обе, не выбран
удобный первый artifact. Rlib/rmeta и executables скопированы вне TTL с hashes.

Первый parser остановился до candidate exec: Cargo-vv stdout содержит build-script
строки рядом с JSON. При точной сверке потребовался также учёт duplicate extern
и обеих форм rustc options. Весь raw вывод и ошибки сохранены; никакого выполнения
candidate при неуспешном verifier не было. Backend не был неоднозначен: связи
сопоставлены по точным путям/suffix, а не по одному имени crate.

`receipt-at-runtime.json` / `verifier-at-runtime.py` — успешный receipt и verifier
**перед** первым тестом/host; SHA256 receipt
`00ef73f1eb732561ebd29457ae64765ec60e559c552d710f7d8636545b20fe7e`.
После runtime verifier hardened: явные exception checks сохраняют fail-closed
при python -O; повторный receipt по тем же logs/artifacts успешен, без новой
Rust-сборки/host runtime. Текущий `receipt.json`, `verifier-revision.json`,
`final-binding.json` сохраняют эту связь. Pure negative controls gate отвергают
libc/override cfg/features/x32/missing raw и в обычном, и в optimized Python.

Retained host ELF SHA256:
`3e27fb3cbb893cf2c0e3983846e5969b37ec246bf2737320ff122595e173dd29`.
Retained standalone test ELF:
`7f54e4fbb30bab7097dc6b6eeaff29918786697c1b09c157573c6e3609e6d7d8`.

## Launch profile и actual результаты

Перед candidate tests и обоими host runs сохранены `/proc/self/status`:
Seccomp0, TracerPid0, NoNewPrivs0, без LD_PRELOAD/LD_AUDIT. Inside supervisor и
actual host snapshots также Seccomp0/TracerPid0. Запуск собственный/известный,
без errno-emulation/tracer и без новых caps. Сами getpid/getppid не заменены
другим syscall. Inherited unknown seccomp profile остаётся unsupported.

Обычный DesktopHost, synthetic ephemeral store, private bus/XDG, собственные
loopback VLESS peers/HTTPS/CA, pinned cores прежних SHA. Supervisor PID1 своей
PID/mount/user namespace с текущим UID переживает host. Namespace и fixtures
оставались живыми все 3 секунды наблюдения, до отдельного reap/teardown.

| Control | sing-box1.14.4 | Xray26.10.31 |
| --- | --- | --- |
| Normal Connected, live negative canary | pidfd not-ready + same identity/ELF + owned listener + rebind refused | то же |
| Explicit Stop | exit/reap/rebind + closed/host exit0 | то же |
| Повторный Stop после Disconnected | повторный ACK, без core | то же |
| Actual host SIGKILL | host wait=-9; core pidfd ready за 0,001804s | host wait=-9; core ready за 0,000959s |
| Observation до выхода namespace | во всех 4 samples: exited, listener отсутствует, порт свободен | то же |
| Cleanup | supervisor reaped core status9, rebind, final children empty | то же |

GREEN verdict **exits** для обоих; первая observation не позже ~2ms после wait
host. Не абсолютный latency budget: это измерение данного запуска. Core уже
завершился сам от armed signal; supervisor не добивал живой core. Held pidfd,
identity-bound аварийный kill/reap и finally остались страховкой ошибки стенда.
`*-green/result.json`, profile files и logs сохраняют raw assertions/snapshots.

## Focused проверки и границы

Rust test ELF запущен только после receipt: 2 passed, 0 failed, 1 ignored
служебная fixture-role (она вызывается отдельным child, не acceptance case).
Проверены live-owner normal kill/wait, already-exited repeated wait и смерть
создавшего OS thread при живом основном процессе. Нет нового unsafe/test-only
pre_exec hook/generic syscall API. Actual normal Stop/repeated Stop проверены
обоими pinned cores, не заменены fixture.

- До первой parent-check / между check и arming / между arming и второй check:
  существующий double-check алгоритм и static proof; детерминированный injected
  runtime **не выполнен**. Для этого потребовались бы неразрешённые hooks.
- Post-arm owner thread exit и whole-host SIGKILL: actual controls выше.
- Prctl errno/EINTR: fail-closed raw errno propagation в exact code;
  actual error injection **не выполнен**. Getppid errno emulation запрещён
  предусловием grant, а не считается обработанным io::Error.
- Drop/await cleanup семантика не менялась; SIGKILL не исполняет Drop и не
  создаёт application ACK. Pdeath не общий descendant fence и не native recovery.
- TGID/thread limitation и unsupported backend/profile contract из proposal
  остаются действующими. Validate/Apply barriers, full P09 и native host-loss OPEN.

Fmt all --check, nsfs all-targets clippy -D warnings и normal-host example clippy
(включая linux lib) — PASS. Ruff/diff-check PASS. Workspace-wide old reducer suites,
100-cycle/P03, Runtime16/worker/native/N5 не запускались.

## Retention и cleanup

`final-binding.json` проверяет unchanged compiled inputs, exact patch after-hashes,
retained ELF/rlib hashes, runtime receipt и отсутствие активных target references.
ELF, rlib/rmeta, source manifests, metadata, raw vv/JSON, profile и runtime logs
сохранены в evidence вне TTL **до** cleanup. Под root flock выполнен штатный
cargo clean (4282 files / 2,2GiB), собственный пустой transient root удалён.
Сборок/host/core/peer children не осталось. Свидетельства исходных RED и proposal
refs не перезаписаны. Нужен independent review нового integrated fix/receipt,
а не повторный прогон старой матрицы.
