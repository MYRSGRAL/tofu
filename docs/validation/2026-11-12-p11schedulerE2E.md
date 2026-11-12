# P11: actual GUI scheduler service Corrupt

База `e46e14157b54267069c3c80c70f6a99f08d0f38f`.

`p11schedulerE2E` до запуска Qt создаёт новый приватный каталог и настоящий
EncryptedSubscriptionRepository, записывает synthetic source и повреждает только
его encrypted payload через sqlite3. Случайный ephemeral key остаётся в процессе
и передаётся существующим HostConfig::with_ephemeral_key; keyring не используется.
Настоящий DesktopHost при scheduler load получает RepositoryError::Corrupt.
Поля HostConfig, диагностическая matrix, pins, capabilities и ABI не меняются.

Production bridge_impl вынесен из main в общий модуль без изменения GUI-методов;
внутренний configure возвращает boxed HostConfig только при повторной настройке,
чтобы test-support сборка проходила result_large_err без suppression.
example использует тот же initialize, dispatch, copy_diagnostics_v2 и shutdown.
QML interposer подменяет только UI для автоматизации, не producer/state/config.
Хороший snapshot поступает от DesktopHost без ручной сборки контекста. Стенд
показывает RU/EN errorText из Main.qml, копирует V2 обычным Bridge action и читает
нативный clipboard через TextEdit.paste. Независимый Python validator принимает
каждый export. Затем копии actual snapshot с stale scheduler generation,
неизвестной причиной и удалённым cause должны отвергаться без изменения clipboard.
Завершение требует actual closed=true после backend.shutdown.

Запуск (binary заранее сохранить вне TTL; TASK — новый каталог):

```sh
unshare -Urnpf --mount-proc dbus-run-session -- env \
 QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software PYTHONPATH=tools \
 python3 tools/tests/check_p11schedulerE2E.py BINARY TASK
```

Evidence: `/home/myrs/projects/yonik/local/validation/p11schedulerE2E/`.

Это один actual service Corrupt case. Полный P11 не закрыт: actual GUI Full,
interval validation, прочие service/storage variants, scheduler owner/counter
exhaustion, helper timeout/denied/owner-loss и drain остаются отдельными cases.
Linux offscreen не заменяет visible compositor gate, Android или физические
G-UI/G-POWER.

Финальные проверки: GUI all-targets clippy Rust 1.85.1 с `gui,test-support`,
`--offline --locked -- -D warnings` PASS; backend library tests с test-support:
69 PASS; финальная GUI example build и cargo fmt check PASS. Final binary actual
acceptance: RU/EN strict V2/native clipboard, stale/unknown/no-cause refusal и
closed=true PASS (`acceptance-final-binary.log`, каталог `final-binary`).
SHA256 бинарника: `51bd6591f65cd82a92ad85106a589f185a68df1c16356d4235fad12d405e8347`.
Python strict export tests 5 PASS, factory tests 2 PASS, syntax PASS.
Ruff в среде недоступен. GUI library test invocation отдельно не линкуется:
FFI Bridge реализуется в executable; неуспешный лог `rust-tests.log` сохранён,
этому прогону PASS не присваивается. Первый clippy выявил большой Err внутреннего
configure; исправление проверено финальным clippy без lint suppressions.
