# Lifetime directory flock при backup

CI PR115 2b1653a упал с Conflict в path_boundaries_and_foreign_staging:250.
Лог не различает directory flock, SQLite и публикацию. Историческая причина
этого CI остаётся UNKNOWN; исправление ниже не объявляется её доказательством.

Независимый controlled fork probe подтвердил: CLOEXEC не закрывает дубликат
до exec, а закрытие родительского File не снимает flock при живом OFD alias.
Root regression детерминированно клонирует directory FD после успешного lock,
первый copy завершает fail:before-vacuum и повторяет его при живом дубликате.
Без исправления — реальный Conflict (RED), с private RAII guard — PASS.
Та же проверка покрывает успешный возврат. Независимый конкурентный владелец
по-прежнему вызывает Conflict; timeout/retry/SQLite/HMAC проверки не ослаблены.

Guard создаётся только после успешного lock и выполняет явный Unlock на всех
выходах. Его lifetime включает cleanup и parent directory sync; EINTR повторяется.
В Drop при иной ошибке остаётся стандартный File-close fallback, как у
существующего LockedDirectory. Hooks и удерживаемые тестовые FD cfg(test)-only.

Rust 1.85.1 --locked: 16 fault tests; 405 Linux lib PASS/29 ignored,
2 maintenance access и 4 storage backup PASS. Pinned all-targets Clippy и fmt PASS.
Evidence, RED/GREEN, сохранённый libtest и source manifest:
local/validation/backup-flock-ci/. Source-before-full записан до полного прогона;
RED выполнен раньше guard, не выдаётся за независимо аттестованную сборку.
Новые CI будут проверяться отдельно; source startup hang по-прежнему UNKNOWN.
