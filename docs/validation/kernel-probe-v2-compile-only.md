# Kernel probe v2: только применение и компиляция

Разрешение `kernel_probe_v2_apply_compile_only` относится к патчу SHA-256
`b978d16487613267f65e1a6f540017d3308c2ef8e18ed8c55b7e4c5d6029f318`.
Применён этот патч с форматированием rustfmt 1.85.1, без изменения семантики.

Адаптер содержит ровно 16 фиксированных invalid-вызовов unshare/legacy clone
для native Linux x86_64 LP64. Новых callers, runnable fixtures, тестов с вызовом
адаптера, зависимостей и интеграции с report/launcher/mint нет. Старые probe API
и TUN не изменены. Errno не предоставляет authority; admission gates закрыты.

Runtime не разрешён. R2 safety review относится к исходникам Linux v6.18;
W3 static predicate proof относится к фактическому подписанному ядру на диске.
Состояние booted memory и runtime patches не проверено. Это не runtime acceptance.

Проверки ограничены yonik-nsfs: cargo check --all-targets, cargo clippy
--all-targets -- -D warnings, cargo test --no-run; toolchain 1.85.1,
--locked, jobs=1. Тестовые бинарники и новые syscalls не запускались.
Локальное evidence: `/home/myrs/projects/yonik/local/orchestration/evidence/kernel-probe-v2-compile-only`;
там сохранены исходный патч, точный применённый и форматированный source,
команды компилятора, exit codes и SHA-256 manifest. Evidence не входит в git.
