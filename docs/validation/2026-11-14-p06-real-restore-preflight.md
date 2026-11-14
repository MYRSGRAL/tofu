# P06 real Secret Service Restore NEW: preflight, НЕ PASS

Base: `8ea826a04a745c8916e14a4329588c75f66b683e`.
Production, Cargo dependencies и исторические A4/private copies не менялись.

Historical reconciliation: `2026-11-11-gui-maintenance.md` явно использует
runtime ephemeral keys; `2026-11-12-format4-secret-service-gui.md` доказывает
реальный backup и live DB reopen, но не restored NEW normal Host.
`2026-11-11-real-secret-service.md` покрывает прежние Missing/Locked случаи,
повторять их не требуется. §6 data-and-contracts требует сохранения custody.

QML MaintenanceView.submit передаёт maintenance_start непосредственно DesktopHost.
HostConfig::new поддерживает отдельный data_dir, startup открывает
subscriptions.sqlite и вызывает production subscription_master_key.
Поэтому focused normal Host witness достаточен для этого узкого gap.

Read-only P02 startup recipe найден в worktree
`/home/myrs/.t3/worktrees/yonik/t3-aef0f1a8-4b0e7a90d95fae43/tools/p02/private_preflight.py`.
Отдельный короткий private KSecretD preflight выполнен: имя отсутствовало до
старта, owner PID 2007150 совпал с управляемым daemon PID; daemon и bus завершены.
Никаких GetSecret inspector, host HOME/keyring, install/download или VM.
Это только доступность runtime, НЕ restore proof.

Новые example/harness подготовлены для normal import/approve, schema3 migration
backup, отдельного seed shutdown, backup→Restore NEW, missing-source negative,
matching request ACK, Closed ACK, fresh normal Host и production offline
original/revision/metadata verification. Python inspector читает только публичные
item paths/Attributes/Locked и owner PID/UID. Эти сценарии ЕЩЁ НЕ ВЫПОЛНЕНЫ.

## Точный blocker и следующий шаг

Один Rust 1.85.1 --locked build через shared slot, jobs1 и root-directory flock
завершился exit0. В конце этой сборки example скорректирован: добавлен необходимый
schema3 backup перед первым approve и отдельный seed stage, чтобы сравнивать
существующий item до/после maintenance. Из-за пересечения редактирования со
сборкой ELF нельзя связать с текущим source. Runtime этого ELF не запускался.
Это ошибка последовательности исполнителя, не production API defect.

Root нужен один новый focused build текущего example, затем запуск harness.
Production changes не предлагаются. Перед build следует записать свежий полный
source manifest; исходники во время build не изменять. После run проверить
source unchanged, identity/count continuity, actual restored data и negative.
Даже успешный последующий запуск не означает полный Linux/P06 acceptance.

Evidence вне TTL в текущем worktree:
`local/evidence/p06-real-restore/`: build.log, initial source archive/manifest,
ELF с суффиксом unbound, final-manifest.json с hashes и явным BLOCKED.
Rustfmt и Ruff check/format текущих файлов PASS; текущий Rust example после
правки не type-checked. PR до runtime/targeted проверки не создан.
