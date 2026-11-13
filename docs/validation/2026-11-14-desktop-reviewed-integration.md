# Интеграция reviewed desktop stack

Source head: `538ff996507ccc2566d9330e5148e7d4d1767587`.
База: actual origin/main `128a070884e82bf21b5c22a74ba908dadd7daf5a`.
Отдельная integration-ветка; старые feature refs и LocalSocks169 не изменены.

| Срез | Exact reviewed head | Состояние этой интеграции |
| --- | --- | --- |
| Maintenance145 | `6e3d0490706c223b90fe6ab773446ec66ffb001f` | incoming, ordinary merge b467e98 |
| Probe152 | `edc301b85afa34fbfb5049276070c10b27bbf125` | incoming, ordinary merge 41faf5d |
| Disk160 | `8fff27dbe58892a6831e9171530f6dad8c9da900` | incoming, ordinary merge fa368da |
| Closure163 | `999662c53cbdb92c009340831aaf69e868c7294e` | incoming, ordinary merge 538ff99 |
| Source150 | accepted main merge `724638ac69b8d7afb1d09020e94c0f09d4d64c35` | уже в базе |
| Fetch149 | accepted main merge `09d53e5` | уже в базе |
| Startup154 | accepted main merge `5fd6153` | уже в базе |
| Clippy162 | accepted main merge `128a070` | уже в базе |

Полные parents и remerge diff сохранены в evidence. Единственные ручные
разрешения — точный union V2 JSON: общие ключи assert equality, итог 166.
Rust match и field logic слиты автоматически, whole-file ours/theirs не применялись.
Новые API и категории сверх reviewed contracts не добавлены.

Maintenance сохраняет primary error precedence при rollback, SourceFailure —
типизированную публикацию, Startup — queued callbacks. Финальный shutdown
сохраняет DiskLogGuard и не выдаёт closed до окончания pending disk drain.

Исторический ledger #163 остаётся frozen: 3 closed / 11 incoming на его старой
базе. Таблица выше — отдельный integration mapping, а не переписывание истории.
P11 full остаётся open до integrated review и CI. Полный GUI runtime не заявлен.
Disk I/O с неопределённым временем syscall остаётся unknown/pending, не closed.

#161 принят в main `a62dc33d73dce9f1d5d3a2bc5ba570d1cbd414fb`. После terminal
первой сборки он включён ordinary merge `35476f7c1969676bd9017681b72a866997b1dc74`.
Delta — четыре attribution/RPC-файла, без desktop/journal изменений. #168 не включён.
Scoped PASS не означает PASS всей CI или kernel acceptance.
LocalSocks100 #169 не повторялся; его evidence не приписывается этому binary.
Probe-v2 proposals, kernel evidence и прочие refs не изменялись.

Evidence вне TTL: `local/validation/desktop-reviewed-integration/` этого worktree.
Source-binding.json привязывает исходники к source head; binary binding сохраняет
SHA-256 реальных test executables до штатного cargo clean.

## Проверки и binding

На исходном integration head `538ff99`, Rust 1.85.1, jobs=1: desktop lib/backend/
startup/thread-start/maintenance/logs targets — 176 PASS; disk event_log — 14 PASS.
Ignored cases не считались PASS. Scoped Clippy all-targets -D warnings и fmt PASS.
Первый запуск subcommands не стартовал из-за stable shared library; явный MSRV
subcommand с RUSTUP_TOOLCHAIN/LD_LIBRARY_PATH прошёл, исходники не менялись.

На обновлённом head `35476f7`: attribution — 43 PASS (включая retained/single RPC),
private D-Bus maintenance_host — 1 PASS, scoped Clippy all-targets и fmt PASS.
Desktop full campaign не повторялась: desktop исходники побайтно неизменны.
Python catalog/V2/closure/maintenance — 22 PASS; RU/EN — 374 пары, 160 ключей, 17 QML.
Ruff отсутствует локально; lint ожидается в CI.

`binary-binding.json` относится только к `538ff99`; `main-update-binding.json` —
к `35476f7` (attribution и private maintenance). SHA-256/logs сохранены вне TTL;
GUI application binary не собирался, runtime claims ему не присваиваются.
