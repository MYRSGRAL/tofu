# A4: фактический nonfinal controller rehearsal

Результат: **BLOCKED**, `final_linux_acceptance=false`. Production admission,
factory/N5 и остальные readiness gates закрыты. Это не Linux package seal.

Выбранный root source: `14229843f9a11bf98ec4f5992df00f3247102ea2`
(merge PR166). Release ID: `linux-controller-rehearsal-14229843`.
Ветка создана точно от этого SHA; более новый origin/main не подменял freeze.
TCB: `/home/myrs/projects/yonik/local/orchestration/release-controller-contract.json`.
Registry: `/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843`.
Никакие прежние registry records не изменялись.

## Реальные операции

Из назначенного worktree выполнены:

```sh
python tools/release_controller.py freeze --release-id linux-controller-rehearsal-14229843 --commit 14229843f9a11bf98ec4f5992df00f3247102ea2
python tools/release_controller.py ingest-cores --release-id linux-controller-rehearsal-14229843
python tools/release_controller.py collect --release-id linux-controller-rehearsal-14229843
python tools/check_package_provenance.py --release-id linux-controller-rehearsal-14229843 --package /home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843/package.tar --inputs /home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843
```

Exit соответственно: **0, 0, 2, 2**. Полные stdout/stderr, source/tree,
recipe, tooling hashes, controller command/exit, core SHA и inventory находятся
в [evidence](../evidence/linux-controller-rehearsal-14229843/).
`source.tar` и оба ingested ELF остаются в canonical registry вне TTL;
точные размеры и SHA записаны в `registry-inventory.json`.

Ingest принял actual fixed Go167 job: builder revision `ce356b452675036fab3388ef64ea03f8f00910ce`,
patch bindings, pinned upstream core commits, ABI manifest и ELF SHA проверены
штатным controller относительно frozen archive. Это не утверждение, что Go ELF
собраны из нового product commit: build provenance сохраняет прежний component
source. Полный collect/check_core/package compliance ещё не выполнен.

Collect остановился на первом `rustc --version` (exit 1):
`the 'rustc' binary ... is not applicable to the 'stable-x86_64-unknown-linux-gnu' toolchain`.
Диагностика подтверждает: системные `/usr/bin/rustc` и `/usr/bin/cargo` 1.99.0
доступны; PATH выбирает rustup proxy и cargo wrapper. Установка не требуется.
Ничего в системном toolchain/PATH пользователя не менялось.

`failure.json` создан controller. Rust-команд сборки выполнено **0 из максимум 2**.
Нет GUI/helper/worker ELF, candidate package, `collected.json` и положительного
compliance/provenance результата. Gate честно вернул `Нет controller-collected build`.
Пакет и положительные attestation не фабриковались.

Повтор этого ID не выполнялся: `LocalRunner` создаёт controller-logs исключительно,
а failure/command records append-only. Продолжение требует root-authorized retry
контракта либо нового nonfinal rehearsal ID с проверенным process PATH, ведущим
к уже установленному system toolchain. Это конкретный операционный blocker,
а не отсутствие проектирования collector. Контракт и producer не исправлялись.

## Сохранность и ограничения

Прежняя ветка PR175 с head `70f501e6eb4c6ed9f98322d2f6233ba0d6d2b6ce`
сохранена. PR171 head `ca801bc1b6916522954b6e97a081b305f7b0bfe7`
и PR147 head `dd78f81c5c95930fad18b91f73f3a03d438e97a9` merged;
FR07 [integrated witness](2026-11-13-fr07-integrated.md) не изменялся.

Собственный transient task (20 MiB / 15781898 logical bytes по failure record)
удалён после сохранения evidence, проверки отсутствия target и cwd процессов,
под exclusive flock корня. Cargo target не создавался, cargo clean неприменим.
Frozen source archive сохраняется в registry. Registry failure.left_task отражает
путь на момент ошибки; текущее удаление зафиксировано в `cleanup.json`.

Host install/upgrade, sudo, Docker, runtime/probes/bootstrap/New16,
cross-UID campaigns и deploy не запускались. Будущие N5 product changes требуют
отдельного final source/build и не покрыты этим rehearsal.
