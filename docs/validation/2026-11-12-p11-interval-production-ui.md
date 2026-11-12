# P11: example interval input → production validator → Main copy

База interval leaf: `9324cb1b0b410787699f6549b533874789e23358` (parent PR132). Scope — Linux.

Штатная SetSubscriptionSchedule принимает u32 >=86400; Store::valid требует ту
же нижнюю границу. Scheduler::set_interval принимает целые секунды от 86400 до
u32::MAX. Поэтому штатный ввод не может вызвать scheduler InvalidLimits:
Bridge/backend invalid_input нельзя выдавать за scheduler companion.

С согласования root добавлен только cfg(test-support) constructor
HostConfig::ephemeral_scheduler_interval_example. Он обязательно создаёт ephemeral
конфигурацию, не имеет Bridge/env/preferences входа и заменяет только Duration
перед существующим production runtime.set_interval. Cause/state/context не
присваиваются. Обычная сборка без test-support сохраняет API/defaults.

Отдельный p11schedulerIntervalE2E создаёт encrypted repository и одну настоящую
SubscriptionSource. Duration::ZERO передаётся настоящему validator; поддерживаемый
контроль — Duration::from_secs(86400). Persisted preferences остаются 86400.
Это example-input proof, не пользовательский ввод запрещённого interval.

Observer check_scheduler_production_ui.py --case interval требует actual reason
scheduler_interval_invalid со stage scheduler_validation. Затем проходит штатные
Settings → Diagnostics → copyDiagnosticsButton.onClicked в неизменённом qrc Main,
читает native clipboard через TextEdit.paste и проверяет strict V2. Supported
контроль требует ноль contexts, настоящий ready и запись в repository. RU case
использует настоящий languageChoice.activated; EN запускается с сохранёнными
preferences после этого lifecycle. Каждый case требует closed snapshot и terminal
Qt exit. SHA256 qrc Main сверяется с исходным Main.qml.

Evidence вне TTL: `/home/myrs/projects/yonik/local/validation/p11-interval-132/`.
Каталоги verified-invalid-ru/en и verified-supported-ru/en содержат actual/final/closed snapshots,
qt.log, strict V2 exports. Бинарник и hashes сохраняются отдельно от Cargo target.

Границы: offscreen не доказывает физические mouse hit testing/compositor gates.
Corrupt/Full campaign не повторяется. Unknown/fence controls не являются actual
lifecycle proof. Полный P11, Android и физические G-UI/G-POWER не заявляются.

Остаток P11 из актуальных validation docs после этого узкого interval-case:
subscription/probe failures; отдельные scheduler service/storage failures кроме
Corrupt; thread-start main.initialize; unknown I/O kinds и drain causes;
actual Exhausted счётчиков/runtime/initialize; live helper timeout/denied/owner-loss
в VM; full Run EACCES вне maintenance admission scope. Maintenance admission уже имеет отдельный bounded proof. Общий P11 index
также требует полный message-key inventory, полноту RU/EN и отсутствие credentials
и управляющих последовательностей во всём экспорте; локальные PASS не закрывают
все producers. Источники: 2026-11-12-p11-scheduler-context.md,
2026-11-12-p11-service-context.md, 2026-11-12-p11-maintenance-admission.md,
2026-11-12-p11-production-ui.md, 2026-11-11-linux-acceptance-index.md.

Проверки: actual Main copy InvalidLimits RU/EN и supported RU/EN PASS;
Rust 1.85.1 desktop test-support lib 73/73 PASS; Python validator/factory/observer
10/10 PASS; pinned Ruff 0.17.4 check и format --check PASS; translations PASS.
Ранние failed supported cases сохранены: observer сначала ожидал diagnostic
generation при успешном старте. Production успешный старт публикует null;
положительный контроль требует ready, одну source и отсутствие errors.

Retained binary build_id: `0.1.0+9324cb1b0b41-dirty`; Rust build inputs
между source-manifest и final-source-manifest не менялись. Harness уточнён
после первых supported failures; финальные verified cases используют его
итоговую версию. SHA256 binary:
`e32c4cd2554f3e0f4b68bfd6379f5082e5ae62a08199835477c45853fc5c26ce`.

Rust 1.85.1 GUI/test-support clippy --all-targets -- -D warnings и
cargo fmt --all -- --check: terminal exit 0. Системные Qt/GCC16 SFINAE
diagnostics не являются Rust/clippy errors и сохранены в clippy.log.

Обычный MSRV cargo check --lib без test-support: terminal PASS.

После terminal capture external refs=[]; штатный cargo clean PASS, собственный
transient root удалён. Единственные нужные binary/evidence сохранены вне TTL.
