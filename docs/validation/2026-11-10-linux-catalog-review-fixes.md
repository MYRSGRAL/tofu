# PR #55: исправление двух P2 после независимого review

Проверяемая база: `57f123bdd6554b39d30df63d8b341ead333953a0`.
Изменены только applications.rs, application_catalog.rs и этот evidence.

Бюджет max_dirs теперь расходуется при успешном открытии корня/дочернего
каталога, включая ожидающие в stack. Проверки бюджета и глубины выполняются
до дочернего openat, корневой бюджет проверяется до directory(root).
Тип записи определяется statat с SYMLINK_NOFOLLOW без открытия каталога.
Открытие сохраняет NOFOLLOW; чтение файлов сохраняет NONBLOCK/regular check.
ENOENT/ENOTDIR при открытии вложенного каталога отделены от ошибок доступа
и других ошибок: последние устанавливают incomplete. Ошибки statat кроме
ENOENT также отражаются в incomplete. Desktop precedence/masking сохранены.

Узкая точка наблюдения desktop_snapshot_with — тот же путь, который вызывает
публичный desktop_snapshot, с подсчётом реальных попыток дочернего openat.
Тест использует 12 пустых каталогов, не тысячи FD. При max_dirs=1 ожидается
0 дочерних открытий. Реальный EACCES проверяется на собственной fixture private
с mode 000; восстановление mode 700 происходит до удаления fixture.
Для root предусмотрен дочерний test executable с UID/GID 65534. Фактическая
проверка проведена под UID 1000; ветка запуска из root отдельно не выполнялась.

RED до исправления (Rust 1.85.1, CARGO_BUILD_JOBS=2):

- `cargo +1.85.1 test -p yonik-linux --lib directory_budget_precedes_child_open`
  — exit 101, счётчик left=12, right=0.
- `cargo +1.85.1 test -p yonik-linux --test application_catalog inaccessible_nested_directory_marks_snapshot_incomplete`
  — exit 101, «Недоступный вложенный каталог потерян без диагностики»;
  перед проверкой incomplete подтверждён настоящий PermissionDenied.

GREEN и финальные проверки:

- `cargo +1.85.1 test -p yonik-linux --lib --test application_catalog`
  — exit 0; библиотека 289 passed, 20 ignored; catalog 9 passed.
- `cargo +1.85.1 clippy -p yonik-linux --all-targets -- -D warnings`
  — exit 0. Первоначальное замечание non_octal_unix_permissions в fixture
  исправлено заменой 0 на 0o000.
- `cargo +1.85.1 fmt --all -- --check` — exit 0.
- `git diff --check` — exit 0.

Общий workspace повторно не прогонялся: исправления ограничены обходом каталога.
Сохранены прежние ограничения API и отсутствие GUI/flow attribution.
Все Cargo-команды выполнялись с CARGO_BUILD_JOBS=2 и собственным
CARGO_TARGET_DIR=/var/tmp/codex-myrs/transient/yonik-catalog-review-bf19/target,
под flock -s корня task. Этот отчёт сохранён вне TTL до cargo clean.
