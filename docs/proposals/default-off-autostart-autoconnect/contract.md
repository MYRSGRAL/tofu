# Linux: раздельные автозапуск и автоподключение

Статус: проект для решения root, НЕ ПРИМЕНЕНО. База `3247e6c`.
Ветка `t3/linux-autostart-contract`; только новая документация в этом каталоге.
Нет runtime evidence, изменения production, установки или записи XDG autostart.
Это не FR-19/P21 PASS и не новый always-on controller.

## Найденный gap и повторно используемые пути

Проверены `crates`, CLI, Qt host/QML, repository/subscriptions, preferences,
платформенные контракты и старые validation. Поиск включал autostart,
autoconnect, start-hidden, into-tray, автозап/автоподкл и фактические поля selection.

| Источник на базе | Установленный факт / граница |
| --- | --- |
| `docs/specs/requirements.md:45`, `routing-and-tray.md:25` | Две независимые настройки, первоначально OFF. Первая запись в разделе рекомендаций; root должен подтвердить объём реализации, а не объявить весь FR-19 нарушенным |
| `docs/specs/data-and-contracts.md:97–99` | Доступ к ключу после reboot/lock требует отдельного platform proof; ephemeral запрещает autoconnect |
| `crates/yonik-desktop/src/preferences.rs:17–40` | Закрытая Preferences v1: engine, port, probe, language, proxy DNS, subscription interval/background; двух bool и last-profile нет. Background subscription не является autoconnect |
| `preferences.rs:51–155` | 4096-byte limit, owner/mode/no-follow, сохранение исходных bytes, compare-before-publication, temp/fsync, Durable/PublishedDurabilityUnknown; это основа, не полноценный межпроцессный atomic CAS |
| `crates/yonik-desktop/src/main.rs:15–82` | Каталог legacy `XDG_DATA_HOME/tofu`; закрытый список CLI, start-hidden отсутствует. Smoke использует ephemeral key и не доказывает normal custody |
| `crates/yonik-desktop/src/backend.rs:1064,1148,1531,2190–2206,3962` | Selected profile — память actor; SelectProfile проверяет текущий source/revision, после старта selected=None |
| `crates/yonik-linux/src/repository.rs:1885`, `subscriptions.rs:683` | ResolvedProfile: profile, server, snapshot_revision, snapshot_generation, source_generation; select_profile(id, snapshot_revision) уже делает resolve того же snapshot |
| `backend.rs:522,4955–5005` | Режимы Local/System, отдельные ручные Connect paths; System идёт через policy/Flight, Local через существующий controller. Engine preference не определяет connection mode |
| `crates/yonik-desktop/qml/Main.qml:16`, `application.cpp:253–310,459` | Окно visible=true; tray.show; show action, watcher unregistration поднимает окно; повторный запуск отправляет show существующему host. Это не скрытый startup |
| `docs/validation/2026-11-11-fr19-tray-private-bus.md` и `2026-11-11-fr19-cleanup-negative.md` | Tray/relaunch/cleanup evidence, не persistent opt-in/autoconnect. Private-bus watcher restart не physical Plasma proof |
| `docs/validation/2026-11-11-real-secret-service.md:48–54,88–92` | Software service/private-session restart доказаны на старом стенде; PAM/login/GUI prompt/KDE Wallet не доказаны |
| `docs/validation/2026-11-11-arch-production-custody.md` | Сохранённые ciphertext и old/new preferences evidence полезны для regression, но не реализуют startup connect |

Итог ограниченного source audit: реализации двух настроек под alias не найдено.
Новая архитектура хранения/контроллера не нужна. Нужны расширение preferences,
постоянный проверяемый target и узкий startup coordinator. Исторический PASS
нельзя переносить на новый head без source/evidence join.

## Контракт хранения и API (предложенные имена)

В существующем preferences.json: новая закрытая schema v2 с `revision: u64`,
`autostart_into_tray: bool = false`, `autoconnect: bool = false`,
`autoconnect_target: null | Target`. Target — метаданные без server/raw JSON/ключа:
`source_id`, `profile_id`, `source_generation`, `snapshot_generation`,
`snapshot_revision`, `engine` (xray/sing-box), `mode` (local/system),
`routing_revision`. Вся запись относится к текущему repository/data-dir;
перенос preferences отдельно от БД не подтверждает binding. При restore/import
не принимать target без повторного явного выбора. Числа в QML DTO передавать
десятичными строками, не терять u64/i64 через JS Number.

Missing v1 поля мигрируют в память как false/false/null/revision=0; исходник
не переписывается при чтении. На первой явной записи сохраняется v2.
Unknown schema, неверные типы/диапазоны, повреждение, небезопасный файл:
сохранить исходник, оба эффективных bool OFF, write blocked, показать причину.
Old reader v1 не обязан читать v2: его deny_unknown_fields должен сохранить
исходник; rollback compatibility проверить явно, не обещать автоматический downgrade.

Предложенный закрытый command:
`startup_preferences_set {request_id, expected_revision, autostart_into_tray, autoconnect}`.
Отдельный `autoconnect_target_set {request_id, expected_revision}` захватывает
только уже выбранный и заново resolved saved profile actor, а не DTO с произвольным
путём/JSON. UI заранее показывает сервер, ядро и mode. Любая смена target требует
явного действия; сохранение target не подключает. Переключение bool не меняет
второй bool и не запускает соединение в текущем процессе.

Actor — единственный writer. CAS проверяет expected_revision, исходные bytes,
сохраняет revision+1 с checked overflow; stale -> conflict без потери другой
настройки. Требуются сериализация writers под существующим single-instance
ownership и повторная byte-check; если несколько host одного data-dir реально
допустимы, до реализации root выбирает общий lock, не выдавать нынешний Store
за линейризуемый CAS. Внешние ручные writers не обязаны соблюдать lock:
обнаруженные изменения блокируют запись, race нельзя скрывать обещанием atomic CAS.
PublishedDurabilityUnknown отображается отдельным ack: файл опубликован,
не откатывать память; автоматические действия в этом запуске запрещены до
подтверждённого durable сохранения/перечтения и reconciliation.

Snapshot: desired bool/revision, effective autostart status
`off|pending|installed|conflict|error|unknown`, autoconnect status
`off|pending|suppressed|blocked|attempted|error`, закрытый reason и target summary.
Ack содержит request_id/revision/outcome/reason, без секретов.

## Startup, tray и XDG

`autostart_into_tray` означает opt-in запуска при входе в desktop session.
Будущий platform adapter владеет только
`$XDG_CONFIG_HOME/autostart/yonik.desktop` (fallback `~/.config/autostart`).
В этом PR никакой записи. Создание лишь после явного durable false->true opt-in;
сам startup не устанавливает entry. Фиксированный executable установленного
приложения + единственный `--start-hidden`, Type=Application, Terminal=false;
никаких sh -c, arbitrary cmd/env, engine-dir, key, user script или credentials.
Точный installed executable/quoting должен дать packaging owner до включения
adapter; при неизвестном install path — blocked, не PATH search fallback.
Проверять абсолютный XDG path, ownership/type/no symlink и ограничение размера;
чужой/изменённый entry не заменять/удалять. Удаление только собственного
проверенного entry по явному OFF. Не трогать system entries и services.

Preferences и desktop entry не атомарная транзакция. Durable intent -> adapter
apply -> отдельный observed status. Ошибка не означает installed; UI показывает
расхождение, явный retry. После crash перечитать оба без overwrite чужого файла.
Существующий собственный entry с desired OFF не даёт разрешения на connect/hidden.

Новый CLI `--start-hidden` — закрытый без значения presentation hint, не право
на connect/установку. При normal startup окно видно. При hint скрыть только
после фактически доступного Qt tray host + готового зарегистрированного icon;
неизвестность/ошибка/таймаут -> видимое окно. После утраты tray показать окно,
сохранив действующую сессию. Повторный запуск только поднимает окно текущего
host, без второго startup attempt. Не добавлять command/env execution authority.

## Autoconnect и Stop

Ровно одна попытка на новый normal desktop-host startup, если bool ON.
Она независима от XDG/hidden: ручной запуск тоже может подключить; autostart ON
при autoconnect OFF запускает только UI. До Ready coordinator ждёт ограниченный
startup barrier; timeout -> blocked с видимым управлением, без фонового retry loop.

Допуск требует normal persistent custody Ready текущего provider owner, saved
profile, совпадение ВСЕХ target bindings/revisions/mode/engine/policy, capability
и обычный Connect admission. На каждом await заново проверить cancellation,
startup generation и binding перед dispatch. Key missing/locked/unavailable,
ephemeral, удаление/обновление source, смена snapshot/policy, pending review,
неподдерживаемое ядро -> fail closed с причиной. Не подбирать похожее имя,
следующий профиль, native JSON или другой mode/engine. Ключ не кэшировать в DTO,
не обходить unlock и не открывать prompt скрытно.

System target проходит ровно существующий policy/helper/N5 admission. Новый
coordinator не mint-ит proof/token, не превращает UI bool в authority и не
использует native system fallback. Без подтверждённого текущего gate — blocked;
Local target не доказывает System поддержку. Автоподключение не делает автоматический
SystemUnlock и не снимает kill-switch protection.

Stop/Shutdown проходят существующую priority generation (backend.rs:553,744,
4194–4235); coordinator получает тот же fence до первого await. Stop отменяет
pending startup и запрещает повторную попытку до завершения этого host lifetime.
Закрытие окна не Stop. Смена сети, tray restart, key unlock/owner reappearance,
ошибка connect/health или cleanup не запускают startup заново. Ручной Connect
остаётся явным действием. Новый процесс при сохранённом ON снова имеет одну
попытку: постоянного запрета после Stop контракт сейчас НЕ задаёт; если root
хочет переживающее reboot «остаться отключённым», это отдельное решение, не
скрытый третий persisted bool.

| Фактическое состояние | Owner / поведение startup |
| --- | --- |
| Blocked protection | System/session owner; показать причину, только явное действие по существующему контракту |
| Unknown | System state owner должен reconcile; не трактовать как Disconnected/разрешение Connect |
| Recovery/recovery_pending | Существующий recovery owner; coordinator подавлен, никаких cleanup/unlock за пользователя |
| CleanupError / startup error / CAS conflict | Соответствующий backend/store owner; окно доступно, reason без секретов; нет retry |
| Уже активная чужая/своя сессия | Reattach/reconcile через текущий owner, не второй controller и не restart |

## Один следующий implementation leaf

**L1: persistent default-off preferences + чистый startup admission planner**.
После одобрения root назначить одному writer `preferences.rs`, новый
`startup_policy.rs` и unit tests (подключение модуля в lib.rs только для planner).
Никаких Main.qml/backend/bridge/CLI/Qt/host adapter изменений в L1; editor writer
и semantic harness сохраняют своё ownership. L1 не объявлять готовой feature.

Результат L1: v1->v2 read migration и durable revision CAS в единственном writer;
чистая функция `decide(input) -> Noop | Wait | Blocked(reason) | Connect(target)`
без I/O и исполнения. Inputs: persisted settings/target, current resolved identity,
custody enum, mode admission enum, tray availability для presentation decision,
startup attempted/cancelled/generation. Не принимать голые readiness bool как
production authority; в planner это лишь снимки проверок, будущий dispatcher
обязан повторить реальные gates. Решение не запускает engine.

Root должен утвердить v2/rollback, semantics Stop/new process и target capture
(последний явно подтверждённый для автоподключения, а не последний успешный
Connect: такого persisted указателя сейчас нет). При несогласии это точечный
design gap; не строить always-on/autoretry вместо решения.

## Проверки и точные неподдержанные ветки

L1 unit matrix: fresh и v1 -> OFF/OFF; четыре комбинации независимы;
v2 roundtrip; unknown/invalid/symlink/oversize preserved; stale revision/overflow,
foreign byte change, write/fsync failure, durability unknown; target fields
по одному mismatch; removed/refresh/reimport; ephemeral/locked/unavailable/stale
owner; Local и System admission deny; Blocked/Unknown/Recovery; Stop на каждом
barrier и stale generation; repeated event -> максимум один Connect decision.
Existing preferences regressions сохранить. Показать тесты и source SHA, запускать
только релевантный cargo test с task transient/flock, без engine/runtime build здесь.

Следующие интеграционные проверки после отдельного назначения: API ack/UI RU/EN,
два переключателя и readback; fake XDG adapter на owned fixture tree (чужой entry,
symlink, crash между intent/apply); CLI allowlist; actual tray/no tray/lost tray;
software restart normal custody и saved profile; Stop race и никакого auto-restart.
Ни fixture bool, ни pure planner не доказывают реальный Connect или actual tray.

Software process/provider restart и LOGICAL boot/network sequencing доступны
как самостоятельные последующие software gates: **не физически deferred**.
Логический boot моделирует новый host, persisted state, задержанные ready/network
и cancellation, не устанавливает service и не выполняет reboot. Старые private-bus
proofs переиспользовать только с явно указанной базой и ограничениями.
Physical PAM/login/logind, lock/unlock после настоящей блокировки, reboot,
реальный DE autostart и KDE Wallet — отдельно NOT PROVEN/deferred platform witness,
не причина отложить всю software матрицу. Locked branch обязана fail closed уже
в software тестах, даже когда физический positive witness ещё отсутствует.

До integration ownership root: CLI/Qt — desktop owner; commands/snapshot и priority
Stop — backend owner; saved profile/custody — repository/platform owner;
XDG entry/install identity — packaging/platform owner. Эта design-подзадача
не даёт им разрешения менять production. Сетевые проверки требуют обоих ядер
на применимых ветках; новая документация не закрывает P21/FR-19 целиком.
