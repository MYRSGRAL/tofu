# Actual legacy LocalSocks: Stop / shared window

Новая измеренная source-привязка: `f0a1d13fb69facfb84bda5f68ba37db6cff92380`.
Это отдельный proof от frozen7b/R2 8de. [Binding](legacy-stop-binding.json)
содержит hashes source/ELF/traces/commands и source scope. Старые binding/archives
не переписаны; весь P05/P10 и N5/native authority не объявляются PASS.

## Causal RED → production fix → GREEN

На source `9e86b16` actual NormalHost/private SS/default DesktopFetch/Xray получил
scheduled held HTTPS response. После Stop: matching successful shutdown owner/ID,
upstream socket closed; новый scheduler имел deadline=null вместо original
61056728114ns (relative monotonic origin). Assertion упал именно на потере окна.
Это actual Stop RED, не вызов replacement helper. Более ранний запуск остановился
на ошибочной fixture due<recorded_wall и не считается target RED.

Наблюдатель уточнён в отдельном test commit `71485e2`: lease trace публикуется
после изменения actual watch; window close имеет begin/success/failure phases.
Первый GREEN candidate сохранил Stop deadline, но остановился на неполном
наблюдении нового lease; это не скрытый PASS. Timeout/assertions не ослаблялись.

Atomic production fix `f0a1d13`: в !quitting priority Stop после того же shutdown
`DesktopSchedule::new()+policy/visible copy` заменены на `scheduled.replacement()`.
Никаких иных production policies/branches не менялось. Signal-before-await,
shutdown/load errors, owner generation и admission сохранены.

## Новый actual witness

Один fixture, два pinned cores, последовательные runs, без transport override:

| Контроль | Xray | sing-box |
|---|---|---|
| NormalHost/private SS, default DesktopFetch, actual pinned core | PASS | PASS |
| Scheduled held HTTPS → Stop → matching successful owner/drain ID + socket EOF | PASS | PASS |
| Original monotonic origin/deadline сохранены через replacements | PASS | PASS |
| Reconnect: actual lease generation 1→2, positive HTTPS refresh/revision=2 | PASS | PASS |
| Нет старого pending/позднего commit; рабочая revision после reopen | PASS | PASS |
| Второй source overdue после original window, без нового запроса/переноса due; reopen due exact | PASS | PASS |
| Normal trace без overflow; own runtime leaders 4/4 reaped | PASS | PASS |

В каждом run 5 HTTP(S) requests: seed, два baseline, held scheduled и positive
new-lease manual. Second-source count=1 — его baseline; persisted due задаётся
через существующий metadata API за исходным 60s окном. После 95s источник
проверенно overdue, count всё ещё 1, due exact/failed=0; окно closed после
исходного deadline. Это различает window barrier и reservation held source.
Старая incomplete body теряет socket на Stop, не превращается в report/commit;
после нового manual revision=2 остаётся неизменной до close/reopen.

Inherited `socks_requests:0` в raw runner result не инструментирует native core:
это оставшийся пустой fixture counter, **не** измерение нуля SOCKS requests.
Native SOCKS count=NULL; production lease/default transport + actual origin
controls доказывают путь, отдельный packet counter не заявляется.

Read-only trace только под nondefault test-support; отдельный HostConfig leaf,
existing with_schedule_observer не изменён. SyncSender/try_send, fixed fields,
sticky overflow/disconnect flag; нет endpoint/URL/credentials/model/state setters.
Actual window opening timestamp взят из того же monotonic input open_window;
эмиссии lease/drain/window идут после actual transitions. ACK success только
после Ok shutdown/close, failure имеет отдельную phase. `drain_id` checked_add,
exhaustion invalidates fixture. Trace не выдаёт authority и не заменяет Host
admission или socket oracle. Capacity=1 actual negative control дал FAIL
`trace overflow/disconnect`; own 4/4 reaped, output не объявлен GREEN.

## Source/build/evidence

Постоянный own prefix:
`/home/myrs/projects/yonik/local/evidence/pr199-legacy-window-v1`.
Final `green3-elf` SHA256:
`3536bcfea41aad722bf8f5f79f47c22a6d4cf20a76d44408a4f552a85a48471c`.
External `binding.json` SHA256:
`d61c3ebb4491aea4dd6d012acd4f6068800ba5c4813dd0ebf26f8ffa98ee712c`.
396 files source manifest/archive проверены pre/post и against exact commit;
это Cargo/crates inputs, не полный root-tree claim. Native inputs не менялись
во время compilation. Один own target/dependency graph Rust1.85.1, incremental
RED/GREEN builds, global slot/jobs1/empty wrappers/directory flock. Source, RED
и final ELF, safe logs/traces/commands сохранены до cargo clean 4752 files/2.3GiB;
own transient root удалён, build/runtime sessions завершены. Старый runtime SS
closure переиспользован read-only с SHA manifest, core SHA проверены до запуска.

Проверки final source: actual GREEN 2; unit 115 PASS/1 ignored; clippy all-targets
-D warnings, default-feature lib check, rustfmt, ruff PASS. Source-bound P11/A08
pure-data gates 8 PASS (log data-checks.log). P11 source update/history отражает
новый blob, old producer_execution остаётся историческим. CI/независимый review
нового source — отдельный gate.

Никаких новых deps/downloads, host session/network changes, N5/NativeRunning,
worker/FR16/credentials production edits. Runtime только own loopback/core/peer/
private bus/SS. P01/P02/другие кампании не опрашивались/не менялись; unit/clippy
overlap и фоновые workers — возможный noise, elapsed не performance/energy proof.
Реальные 60s window относятся к software scheduling, не hardware/power PASS.
