# P11: maintenance admission и fence replacement

Linux; дополнение к scheduler commit b2e7e1126f6b7cc5c3f81b271404b83b388fac31.
Полный P11 не заявляется. Android, физические G-UI/G-POWER отложены.

Controller возвращает AdmissionError до явного отображения в прежний V1 key.
Различаются Busy, Revision, Identity и InvalidInput. Старые validate/begin
сохраняют строковый интерфейс для существующих потребителей. Production supervisor
использует begin_typed, строки не разбираются.

MaintenanceAdmission добавлен в закрытый V2 Stage; matrix и Rust validator
согласованы. Supervisor фиксирует busy/unavailable/draining/stale, отсутствие
handles, scheduler replacement failure и refusal после reopen failure.
Startup failure сохраняет startup ACK. Последние два общих refusal создают
также соответствующие Error/Connection contexts, поскольку V1 уже выставлял
эти поля. Остальные причины разрешены только в MaintenanceAck.

ACK сохраняет V1 request_id, phase и error. V2 использует canonical decimal
ID/generation и params {}. Повтор matching Start во время job возвращает текущий
ACK и очищает контекст чужого refusal. Отмена удерживает capacity до matching
drain; terminal остаётся связан с единственным join handle.

Actual tests: Host + encrypted repository дают stale revision и повтор ID;
private leaf gate проверяет sibling cancellation/refusal при drain и сохранение
lock до join; реальный reopen fault делает Host unavailable и каждый новый
Start/Cancel получает matching busy ACK с typed reason. Startup fault проверяет
каждый matching startup ACK. Изменённые ID/generation/slot отклоняются целиком.

P2 regression использует настоящий scheduler Full поколения A=77 при загрузке
двух persisted subscriptions в capacity=1, затем настоящий backup/reopen и
успешную загрузку replacement B. Production accept_maintenance_owner обновляет
scheduler_generation. Сохранённые error/source contexts A после принятия B
отклоняются. RED до обновления fence: assertion «stale contexts A must fail…»;
GREEN после исправления в той же точке принятия owner.

Evidence вне TTL: /home/myrs/projects/yonik/local/evidence/pr128-maintenance-admission.
Здесь сохраняются RED/GREEN логи, actual snapshots и результаты Qt clipboard.
Для clipboard replay используется production DesktopBridge, RU/EN mapping и
реальный Qt clipboard; неверная generation/channel и отсутствие contexts
должны оставить clipboard без изменения.

Ограничения: subscription/probe/service-storage, unknown I/O/drain и main
thread-start gaps остаются. Full Run EACCES не входит в этот scope. Pins,
capabilities, helper/native ABI и зависимости не менялись.

Проверки: Rust 1.85.1 desktop lib — 61/61; private D-Bus maintenance_host —
1/1; clippy --features gui --all-targets -- -D warnings — exit 0;
cargo fmt; Python diagnostic validators — 5/5; RU/EN translations — pass;
Ruff format/check нового clipboard runner — pass. Production Qt clipboard:
5 actual snapshots × RU/EN — pass, negative clipboard unchanged — 5/5.
Системные заголовки Qt/GCC выдают C++ warnings; Rust/clippy ошибок нет.
