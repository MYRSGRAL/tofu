# Проверка staged carrier

База исходного proposal: 3247e6c32a5ed83ec5d812dfc1f70695d3f2b3b4.
Frozen #204 e1970210190b7e89ab9ffd06a0a33203dec82d85 подключён обычным merge
2c7fac53566376f053a4ba769215d1fe09b625d5. Это dependency, не завершение его review.

Разрешённый leaf **COMPILED / TESTED** только за `cfg(test)` / existing
nondefault `test-support`. Production transport/dispatch/forwarding/exec
**NOT REGISTERED / NOT EXECUTED**. Данные Prepared в тесте синтетические,
но сам Rust тип существующий настоящий Prepared, без FixtureHandle.

Rust 1.85.1, --locked --offline, jobs=1, wrappers unset. Один общий transient target,
BSD shared flock корня `/var/tmp/codex-myrs/transient/fr16-carrier-209` и
`/home/myrs/projects/yonik/local/orchestration/with-rust-slot` на весь build/run.
Команда: `flock -s <root> <with-rust-slot> bash local/fr16-carrier-209/build.sh`.
Рецепт `build.sh` сохранён рядом; его выполнение требует предварительного
создания собственного task root. Он не делает install или production регистрацию.

Первая попытка no-run завершилась compile error в новом тесте (`mkfifo` вместо
имеющегося `mkfifoat`) и unused import после переноса common types. Диагностика
сохранена в initial-compile-error.log. После исправления в том же target выполнена
одна успешная focused test compilation; повторный freeze включает также проверку
borrowed CLOEXEC до owned duplication. Исходники после freeze не редактировались.
Отдельного red test execution не было; не заявляется TDD red/green proof.

- Feature lib test ELF: 8 carrier tests + 2 PreparedOutput tests, все passed.
- Frozen #204: 2 V3/legacy/resource codec tests + 2 exact golden tests, все passed.
- Всего исполнялось 14 tests, 0 failed/ignored; shell/native/systemd/bus tests
  из остальных 593 (всего 607 compiled unit tests) не запускались.
- clippy `-p yonik-linux --all-targets -- -D warnings`: PASS для default;
  та же команда с `--features test-support`: PASS.
- `cargo +1.85.1 fmt --all -- --check`: PASS.
- Source hashes проверены после build и после clippy/fmt: все совпали.

Тесты используют только собственные pipes, named FIFO в locked task root,
файловые FD/type/flag операции и zbus DATA serialization/deserialization.
Нет Connection, bus, systemd, native engine execution, probes или host services.
Для негативных UNIX_FDS/count используются настоящие безопасно построенные DATA
messages с 0/1/лишними/alias handles; builder сам согласует объявленное и фактическое
число FD. Отдельно повреждённый transport header с расходящимися declared/actual
FD не forge-ился (zbus требует unsafe для raw Message). Decoder проверяет оба
числа независимо; transport-level malformed receive proof остаётся будущим gate.
Нет нового wire protocol model: V3 checker/golden из frozen #204 не менялись.
Старые source-mutant wrappers не запускались и не заявляются proof нового carrier.

Вне TTL, не в Git:

- `local/fr16-carrier-209/artifacts/carrier-tests` — сохранённый ELF (196 MiB).
- `local/fr16-carrier-209/source.tar.gz` — source closure: crates, tests, Cargo inputs,
  docs/specs, то есть также frozen fixtures и внешние ABI include inputs.
- `local/fr16-carrier-209/source.sha256` — тот же полный manifest.
- Полные no-run JSON/исходная диагностика также в local/evidence файлах данного worktree.

`artifacts.sha256` содержит контрольные суммы сохранённых ELF/archive/manifest;
`source.sha256` — проверяемый manifest исходников. Артефакты принадлежат этому
worktree и сохраняются до его удаления владельцем; это не единственные копии
исходников (изменения коммитятся в task branch).

После завершения всех команд и выхода build процесса выполнен `cargo +1.85.1 clean`
под теми же root flock/build-slot: удалено 4111 файлов / 2.2 GiB. Единственный ELF
и source archive сохранены заранее; пустой собственный task root затем удалён.
