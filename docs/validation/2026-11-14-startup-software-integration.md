# Раздельный startup: software integration

Реализован утверждённый root scope поверх принятого L1 (#208). Две независимые
настройки изначально OFF. По уточнению root функция входит в полный Linux
acceptance scope (`requirements.md:45`, `routing-and-tray.md:25`). Прежнее снятие
optional workflow с critical path не отменяло требование. Исторические результаты
FR-19/P11 этим уточнением не расширяются.

## Поведение и границы

В «Настройки → Запуск и обновления» отдельно включаются запуск при входе и
автоподключение. Выбранный сохранённый профиль, ядро, режим и routing revision
подтверждаются отдельной командой. Сохранение не начинает соединение.
`startup_preferences_set` и `autoconnect_target_set` используют строковые
request ID/revision; stale CAS возвращает ошибку без изменения другой настройки.
UI содержит RU/EN, pending/conflict/error и durability-unknown состояния.

Preferences v3 — закрытая схема. v1/v2 мигрируют только в память; неизвестный
исходник сохраняется, запись блокируется. Старый closed-v2 reader отвергает v3.
Target дополнен identity: canonical DB path, inode/device БД и каталога, существующий
несекретный DB identifier. Replacement/restore не подтверждается совпадением одних
profile metadata. Нужны новый Host и явное подтверждение target. Ни ключ, ни schema
БД не меняются. Все preferences writers удерживают общий data-dir writer lock
до publication/fsync. Некооперирующему same-UID writer не обещается линейризуемость.

При normal startup выполняется один ограниченный resolve/revalidation barrier.
Проверяются normal custody epoch, repository identity, все target metadata,
routing/engine/mode и generation. После scheduler initialization target повторно
resolve-ится непосредственно перед единственным `Command::Connect`.
Stop меняет generation независимо от actor; queued startup снимается до await.
Поздние network/key/tray события не создают новую попытку. Новый Host — новая
попытка. Ephemeral, неизвестный owner, pending review и несовпавший target закрывают
startup без другого профиля/ядра/режима и без unlock.

Callgraph:

- Normal open → `subscription_master_key_with_owner` → существующий
  `subscription_master_key`/`load_key`; старые Missing/Locked/create fences сохранены.
- Read-only `CredentialOwnerGuard` устанавливает NameOwnerChanged watch до credential
  call, фиксирует bus ID и unique SS owner; disconnect/owner replacement необратимо
  закрывают epoch. Из-за public API secret-service 4.0.0 это отдельная реальная
  connection к той же выбранной шине, а не connection object самой зависимости.
  После await проверяются owner fence, bus identity, generation и repository binding.
  Guard не является permit; plaintext fallback и новый native caller не добавлены.
- System reconcile → существующий `Flight::start(Operation::State)` → обычный
  `SystemSession::state`. Только NoManagedPolicy допускает продолжение. Unknown,
  Recovery/Blocked/активная сессия закрывают попытку. Вложенный read-only Flight
  отменяется при deadline/drop.
- Startup envelope → прежний match `Command::Connect`: Local session или прежний
  PolicyConnect Flight. Все текущие routing/helper/N5 admission остаются на этом
  пути; planner не создаёт proof/token и не разрешает SystemUnlock.

`--start-hidden` — только hint. Обычный запуск видим. Скрытие требует actual SNI
registration: зарегистрированный item должен принадлежать PID приложения, что
проверяется read-only DBus query с deadline и ограничением списка. Qt может иметь
отдельную SNI connection. Отсутствие/утрата трея, blocked startup и ошибки поднимают
окно. Duplicate process посылает только существующий show IPC, не новый startup.

Приватный XDG adapter управляет только `autostart/yonik.desktop` с фиксированным
`Exec=/usr/bin/yonik-desktop --start-hidden` из packaging/arch/PKGBUILD. Нет PATH,
shell или env execution. No-follow/owner/mode/size/content checks сохраняют чужую
запись. Сначала durable preferences, затем apply. PublishedDurabilityUnknown не
выдаётся за rollback и не разрешает автоматические действия. Config root должен
существовать и принадлежать текущему пользователю без group/world write.

## Исторические проверки frozen cfb1664

Rust 1.85.1, `--locked`, jobs=1, собственный transient root под flock и общий
rust slot. До runtime сохранены исходники, ELF и hashes вне TTL. После последнего
изменения только C++ выполнялась адресная GUI пересборка; Rust/Host inputs не менялись.

- Library: **131 PASS, 1 ignored**.
- Startup settings integration: **1 PASS**: четыре комбинации bool, CAS, typed IDs,
  ephemeral capture refusal, отсутствие немедленного Connect, Stop при restart.
- Credentials private bus: **6 PASS**: старые отказы и owner replacement/late-ready.
- `clippy` desktop/linux GUI/test-support, all-targets, `-D warnings`: PASS.
  C++ Qt/GCC header warnings остаются отдельными предупреждениями компилятора.
- Rustfmt, Ruff format/check, RU/EN gate и diff check: PASS.
- Own XDG fixtures: fixed entry, enable/disable, foreign/symlink preservation,
  publication/fsync unknown. В настоящий host autostart ничего не устанавливалось.
- `startup_software_proof.py`: normal Host с настоящим private gnome-keyring и
  сохранённым локальным VLESS profile; **xray-26.10.31 и sing-box-1.14.4 PASS**.
  Новый процесс сам подключился через обычный Connect; получен один запрос к
  собственному HTTPS endpoint с нормальной TLS-проверкой. Stop, Stop-before-barrier,
  SS owner loss + late-ready и repository replacement — PASS, без rearm/fallback.
  Readiness peer проверялась через его собственные socket inode, не TCP probe.
- `startup_ui_proof.py`: **5 PASS**: settings RU/EN при 380×600, no-tray visible,
  ready-tray hidden + duplicate show, tray-loss show, blocked-startup visible.
  Это настоящий Qt/private SNI interface witness, не physical Plasma.

Воспроизведение: сначала build/retain `startup_host` example с feature
`startup-diagnostics` и GUI binary; затем оба proof tools с `--binary` retained ELF
и новым `--output`. Software tool дополнительно требует `--engines` с pinned cores
и `--keyring` с retained gnome-keyring runtime. Root каталога fixture создаётся самим
runner; HOME/XDG/private buses не используют пользовательскую session.

Локальная evidence: `local/validation/startup-integrated/`: candidate5 library/
credentials/clippy, final/runtime-v4 и source/harness manifests; gui-final source,
ELF и пять UI controls. Это отдельные точные source/ELF joins, а не перенос старого
ELF на изменённый backend. Ранние RED/harness logs сохранены отдельно.

Physical login/PAM, reboot/lock, настоящая desktop XDG installation и Plasma не
проверялись. System/N5 runtime campaign не выполнялся; Local proof не доказывает
System поддержку. Нужен узкий root review изменённого backend и непосредственно
затронутого P11 effective binding перед интеграцией с отдельным root fix.

Cleanup: после проверки отсутствия активных owned cwd/exe/fd refs `cargo clean`
удалил 13962 файла / 7.7 GiB. Собственный transient root удалён; текущие ELF/source/
logs retained в workspace без TTL. Чужие cache/build каталоги не очищались.

## Correction после CI и R2 review

Разрешённым ordinary merge включён PR213 exact head
`0bb3e86e22d7bf6709c90453e9f8619b60a59ee4`. Port/domain/editor/typed P11 changes
сохранены из root; не переопределялись в startup fix.

CI Rust failure `health_target_settings_are_validated_without_disclosing_url`
на cfb1664 происходил из-за короткого canary `hidden`: он совпадал с публичным
JSON key `start_hidden`. Fixture теперь передаёт уникальное приватное значение;
raw assertions на отсутствие домена, query key, реального query value и userinfo
сохранены, проверка файла усилена. Defaults/admission не изменялись.

R2: `SaveError::Conflict` отделён от I/O. Current preferences читаются и
валидируются под common writer lock; только valid owned snapshot обновляет
Store original и Actor preferences/startup values/revision. Текущие operational
engine/port/probe inputs не переключаются read refresh. Нет автоматического write
retry, XDG apply, Connect/rearm или rebinding target/owner. Unknown/invalid и
symlink не становятся fresh snapshot; при mixed I/O original сохраняется.
Следующий явный запрос использует актуальный revision и сохраняет чужие поля.

Новые адресные проверки (Rust1.85.1 offline/locked/jobs1, frozen source и retained
ELF до исполнения): privacy 1 PASS; Store conflict/mixed/foreign 3 PASS; Actor
explicit retry/no-rearm 1 PASS; scoped preferences regression и catalog parity —
17 PASS и catalog parity 1 PASS. Финальные source/ELF/logs находятся в
`local/validation/startup-integrated/r2-correction/final-receipts/`. Desktop
all-targets clippy `-D warnings`, rustfmt, Ruff, source catalog coverage (2 PASS)
и translation tests (18 PASS) пройдены отдельно от исторического whole-runtime
campaign. Старый XDG apply receipt инвалидируется при refresh; unknown/error
не снимаются чтением. Добавлены только semantic diagnostic catalog entries и
source catalog tests. Effective P11 ledger не редактируется: root выполнит union
review/rebinding отдельно, без присвоения исторического ELF новому source.

Новый whole-head runtime/physical/N5 proof не заявляется. Исторические runtime
результаты выше остаются привязанными к cfb1664; correction использует только
адресные тесты конкретных failure/review findings.

Correction cleanup: после проверки отсутствия active owned refs сохранены
source/ELF/logs `r2-correction/final-receipts`; own `cargo clean --offline` удалил
4660 файлов / 2.3 GiB, transient `startup-r2-1108` удалён. Новых runtime campaign
повторений не выполнялось.
