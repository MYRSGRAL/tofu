# Maintenance boundaries: причины и drainage

База: `cceede520842a17e4ef548807f63f64b2be19226`.
Ветка: `t3/p11-maintenance-causes`.

`run_with_leaf` использует `shutdown_report`: admission failure запрещает copy,
но подтверждённый terminal drain остаётся подтверждённым. Все владельцы закрываются
до выбора результата. Единственная drain-причина выбирается scheduler → service →
repository; access/copy/reopen/replacement остаются первичными относительно
вторичных close failures. При ошибке replacement rollback закрывает service и
repository, результат закрытия влияет на drained.

Добавлены семь закрытых причин, согласованных в задании. Только существующие
CLEANUP_FAILED/cleanup_failed и STORAGE/storage, MaintenanceDrain/MaintenanceReopen,
MaintenanceAck, пустые params. Catalog, зависимости, native ABI и public hooks
не менялись. JoinError преобразуется в MaintenanceTaskFailed без выдуманных деталей.

Проверки Rust 1.85.1, locked, jobs=1, with-rust-slot, BSD flock task root:

- RED: исходный run_with_leaf с новой регрессией допускает copy после admission
  failure; 13 passed, 1 failed. Python matrix RED: отсутствовали семь причин.
- GREEN focused: 14 passed. Финальный desktop lib: 97 passed, 1 ignored.
- Реальные admission/reload failures через повреждение payload приватной fixture;
  replacement через недопустимый интервал; проверены copy priority и повторное
  открытие БД после rollback. Реальные backup/restore сохраняют запись и owner.
- Реальный JoinError и V2-контекст всех семи причин; подмена request_id,
  generation, slot, code и stage отвергается. Существующая Rust/Python matrix parity.
- Python: 16 passed. rustfmt --check, git diff --check, clippy -p yonik-desktop
  --all-targets --locked -- -D warnings проходят.

Ограничения: actual GUI для каждой причины не запускался. Ошибки close не
инжектировались через новые public hooks. Существующий private-D-Bus integration
maintenance_host прошёл backup/restore assertions, затем упал в очистке:
remove_file получил каталог presets (maintenance_host.rs:85). Этот отдельный
дефект не исправлялся. Ruff отсутствует в окружении.

Локальные логи, финальный тестовый ELF и SHA256SUMS сохранены в игнорируемом
`local/validation/p11-maintenance-causes/` этого worktree. SHA256 ELF:
`ab38d4abeb43645adf5545789db19210c34d422bd79dbc80e05f878160d18e97`.
Transient root `/var/tmp/codex-myrs/transient/p11-maintenance-causes` очищается
после сохранения evidence, без удержания общего build slot.
