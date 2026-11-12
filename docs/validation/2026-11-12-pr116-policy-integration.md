# PR116: интеграция PolicyApply из main

Ordinary merge exact main `3f73b6abbc15517f7c1a498fd0255a62bd490f0e`
(PR113 `e602e9e424030800ac027859b4679b7b27bcfe25`) в
`b283c0e4f5e19e3fd60f43a76bf8ab298192b70b`. Worktree перед merge чистый.
Текстовых конфликтов нет; PR128 не включён.

Сохранены P17 CAS/ACK/busy/cancel, maintenance controller, invalid_ip catalog,
PolicyConnect/inline flight с revision/source fences, typed V2 и SourceChange ACK.
Общий predicate отмены inline flight до первого await дополнен SourceChange,
PresetActivate и PresetRollback; последние изменяют routing revision.
Отдельный unit test подтверждает эти три команды и исключает PresetCancel.
Активный неподдерживаемый preset остаётся отклонён в inline policy capture;
capabilities, pins и ABI не менялись, system_tun=false не пересматривается.

Логи, свежий GUI-бинарник, SHA-256 исходников/бинарника и toolchain сохраняются вне TTL:
`/home/myrs/projects/yonik/local/validation/pr116-policy-integration/`.
Cargo только через with-rust-slot, Rust 1.85.1, jobs=1, один процесс задачи;
target в собственном transient task-root под flock.
Android и физические G-UI/G-POWER отложены. Неизменённые внешние native campaigns
не повторяются.
