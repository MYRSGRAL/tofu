# Синхронизация PR108 без физических измерений

Исходный HEAD: `5ae2b6bffa6b7f5a25197ef125f41500c6cd2ca7`.
Обычный merge parent `ea49d1e185e94b4c70bd75c83656ec9dd4c10e46`
создал `7a88bac`; main `473a53b902d3a6e3ef26f286916ed1018219bc00`
уже является его предком, дополнительный merge не потребовался.
Rebase/reset не применялись. Production backend/helper/native/import не редактировались.

Проверки: 23 Python harness regression PASS (19.885 s), 7 counter tests PASS,
CLI help PASS, py_compile PASS. Harness проверяет PID lifetime, запрет group
signals после внешнего reap, cleanup/cancel/timeout, bounded stdout/stderr и oracle.
Python 3.14 выдал ResourceWarning в тесте внешнего waitpid; тесты прошли,
проверки отсутствия чужих group signals сохранились. Это предупреждение не исправлялось.

`no-physical-dryproof.json` создан fake child с подменёнными session/counter
источниками: проверяет 10 действий и normal exit, не физический benchmark.
`valid=true` относится исключительно к синтетическому протоколу.
G-UI/G-POWER, Android, GPU и physical performance acceptance отложены.
Старый availability.json не перезапускался.

Rust: slot-wrapper, jobs=1, shared flock task-root, собственный transient target.
Desktop: fmt 1.85.1, clippy 1.85.1 -p yonik-desktop --all-targets --locked -- -D warnings PASS.
Benchmark model: fmt/clippy all-targets/test на 1.97 PASS; его существующий
MSRV 1.92 и pins не менялись. Сборки GUI не запускались.
Ruff не установлен; новые Python исходники при синхронизации не менялись.

`manifest.json` фиксирует exact source HEAD и SHA256 исходников/lockfiles;
последующий commit добавляет только evidence. Полные Rust логи, dryproof,
CLI help и counter tests приложены. После terminal проверены внешние fd/cwd/exe
ссылки на task-root, выполнен cargo clean; собственный task-root удалён.
