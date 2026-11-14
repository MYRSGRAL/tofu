# A4: связанный package lifecycle с двумя production-created schema4 профилями

Продолжение **после 17c9dcbc586d2bcf3a70f66ca200d9acb8ccd5ee**, same PR181/task.
Этот run использует legitimate pre-existing production-created userdata:
GUI Backup→text/file import→review→Save→new-GUI reopen из предыдущего run3.
Это **не fresh empty install** и не повторный synthetic sentinel proof.
Исходная единственная retained-копия не изменялась; `original-retained-check.json`
подтверждает полное сравнение storage/config и opaque data/password, mode0700.

Source `14229843f9a11bf98ec4f5992df00f3247102ea2` и candidate
`linux-controller-rehearsal-14229843-msrv85-mtree4` неизменны. Новых build/download
нет; production/controller/recipe не менялись. Старые records не переписаны.
`final_linux_acceptance=false`, production factory CLOSED; **full A4 не закрыт**.

## Actual результат

| Шаг | Package / GUI | Userdata / custody |
| --- | --- | --- |
| Own-root install old на существующие данные | pacman exit0, old normal GUI показывает storage refusal на schema4 | Исходный source142 guarded witness exit0: два профиля |
| Upgrade old→candidate | pacman exit0, normal candidate PID69 показывает text и file профили | До/после фазы witness exit0; bytes/modes/UID/GID storage/config/data сохранены |
| Rollback candidate→old | pacman exit0; normal old PID124 показывает «Ошибка локального хранилища» | До/после package фазы witness exit0, userstate bytes равны; old GUI не прочитал schema4 |
| Возврат candidate | pacman exit0; new normal candidate PID170 снова показывает text и file | До/после package фазы и после normal reopen witness exit0 с теми же оригиналами/revision/metadata/item |
| Uninstall candidate | pacman exit0; desktop/helper/worker ELF отсутствуют | До/после фазы и финальный retained source142 witness exit0; вся сравниваемая userdata побайтово сохранена |

Фазы фиксируют **реальные разные package versions**:
`0.1.0.r20261110-2 → 0.1.0.r20261111-1 → 0.1.0.r20261110-2 → 0.1.0.r20261111-1 → removal`.
`vercmp(old,candidate)=-1`, metadata сохранена в `package-versions.json`.
Имя `candidate-reinstall` обозначает возврат candidate после downgrade;
это не same-version reinstall, в pacman log actual `upgrading yonik`.
Qkk/MTREE и старый empty-data lifecycle повторно не запускались.

Каждая phase JSON содержит before/after file hashes, permissions, UID/GID,
opaque keyring/password byte-equality boolean, Secret Service metadata и два
результата существующего guarded witness. **До byte comparison** GUI закрыт
через tray Quit, witness subprocess завершён, inspect соединения закрыты через
accepted `closing` fix; own descendants не держат storage FD (`readers_before=[]`).
После package операции bytes сравниваются **до** нового API/SQLite open.
Поэтому последующее открытие witness не маскирует package writes.

Witness переиспользован из retained ELF, SHA256
`a059c0572bcb8df741dba63fa088ee40b43260141bf122582954e9e6bfc0207e`.
Каждый успешный результат имеет `count=2`, `original_matches=true`,
`revision_matches=true`, `metadata_matches=true`, `same_unlocked_item=true`,
`provider_uid_pid_verified=true`. Guard проверяет private namespace/root/data,
nonce и owner UID/PID. Для всего run provider PID4 UID1000, unique owner :1.1,
один unlocked item `/org/freedesktop/secrets/collection/login/1`.
Fixture использует только public identity/SearchItems, не вызывает GetSecret
напрямую и не экспортирует ключ/keyhash/password. Ключ читает исключительно
существующий production credential API witness (`create=false`). Никаких новых
профилей, ключа или конфигурации в positive path этого run не инжектировалось.

## Точный rollback gap и нормативный контракт

Actual normal old GUI дважды показывает storage error, не готов к чтению профилей.
Снимок `04-old-rollback-schema4-refusal.png`, `old-rollback-normal-refusal.json`.
Embedded old revision `eec7a56b6b03f2b022a87682f9097ce78b61fa0b`;
его repository source прямо отклоняет `user_version > 3` как UnsupportedVersion
**до** migration/WAL pragmas. Точная UI-ошибка generic storage, typed old runtime
context не выдумывается; source guard подтверждает границу версии.
Основной DB hash, права/UID/GID, конфигурация и opaque keyring/password неизменны.
После old GUI закрытия исчезли **нулевой WAL и SHM**; полное дерево после GUI
поэтому не объявлено byte-identical. После этого source142 API снова доказал
оба original/revision/metadata, candidate normal reopen тоже прошёл.
Никакого restore/forceversion/drop профилей или обхода backup admission нет.

Нормативный `docs/specs/data-and-contracts.md:103`:
«Старое приложение не открывает новую схему на запись»; откат ядра и схемы —
разные операции. Он не обещает чтение schema4 приложением schema3.
`packaging/arch/README.md:106` описывает предыдущий PR119 сценарий чтения
**старого** профиля при rollback и refusal unsupported preferences; этот proof
не распространяется автоматически на новые format4 profiles.

**Разделение для root:** package rollback сохраняет новые данные и ключ,
возврат candidate восстанавливает штатное чтение без восстановления БД — GREEN.
Работоспособность old GUI с двумя schema4 профилями — фактически недоступна;
если original A4 требует именно её, этот gate не пройден. Это не основание
наугад менять source142 или обещать backward compatibility. Для такого требования
root должен определить supported rollback compatibility contract и подходящий
реальный old artifact; никаких новых build/package variants в этом run нет.

## Hook, dependency и trust scope

Pacman root/dbpath/log/cache/gpgdir/hookdir находятся внутри одного собственного
root. Same-UID user namespace с ранее разрешённым own-root mapping, отдельные
mount/PID/net namespaces и private bus/display; host writable home/etc/var/run
не bind-ились. `-dd` сохраняет ранее принятый runtime-closure fixture scope:
это не доказательство автоматического dependency resolution pacman. Реальные
normal ELF исполняются с прежней проверенной runtime closure; новых deps нет.

Реальные post-transaction hooks: sysusers пишет только own-root passwd/group;
desktop MIME cache обновляется. Создание записи UID999 в own passwd не является
запуском процесса foreign UID. Manager и system bus reload scriptlets явно
сообщают `Skipped: Running in chroot`. Fake executables/manager не использованы.
**Live PID1 daemon-reload остаётся неподтверждённым**: own PID1 здесь bwrap/worker,
не установленный systemd с рабочим cgroup manager. Проверка live ветки потребовала
бы отдельного заранее согласованного installed manager стенда/контракта;
host unit/service activation и новые privileges в текущем scope запрещены.
Offline skip не засчитывается как live PASS.

Прежние trusted runtime inputs проверены по cached SHA; нет network/download.
Два accepted Yonik archives SHA-pinned; используется прежний rooted
LocalFileSigLevel=Optional TrustedOnly. Новой detached signature этих архивов
не создавалось и её наличие этим run не заявляется. Package hashes:
old `e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0`,
candidate `26a013747e4c1dbb95e7290cd37b981f5dc7ef891a8865897f09fc45bacd0237`.
Source/tree/package/witness проверены launcher до effects. Итоговая проверка
frozen/COLLECTED lineage и всех frozen controller tooling SHA сохранена отдельно
в `final-lineage-check.json` с честным указанием времени проверки.
Actual installed candidate three ELF hashes/modes/owners совпали с archive;
`candidate-installed-elfs.json`. После uninstall все три отсутствуют.

## Сохранение, cleanup и NEXT

Evidence: `docs/evidence/a4-nonempty-lifecycle-142/run1/` — команды, PIDs/exits,
phase JSON, screenshots, guard/witness metadata и original-state receipt.
Тяжёлый setup/raw log без изменения bytes перенесён outsideTTL с SHA receipts.
Все четыре GUI завершились штатно exit0. Worker дождался всех managed children;
`guest/exits.json`: все exit0. Own directory flock удерживался с первого
использования transient до сохранения state и cleanup. Root удалён;
`final-cleanup-check.json` подтверждает отсутствие cwd/exe/fd/maps references.

Обе own retained копии mode0700 сохранены и **не коммитятся**:

- прежняя единственная: `local/validation/a4-userdata-142/diagnostic-a4-backup-cause-5thkiqwh` — неизменна;
- результат после lifecycle/uninstall: `local/validation/a4-userdata-142/retained-a4-nonempty-qdcjm959` — 112 KiB, два профиля, schema4 и private keyring.

NEXT root: принять отдельный NONFINAL file/data continuity proof либо явно
уточнить требование чтения schema4 old GUI; live PID1 hook остаётся отдельным
remaining gate. Не повторять GREEN GUI/API/data lifecycle ради него.
Full A4/native Linux seal не выставлен; finalLinux=false/factory CLOSED.
Обычный commit/push PR181, без history rewrite/watch.

Проверки этого delta: 5 A4 Python tests PASS, ruff check/format PASS,
staged diff-check PASS. Новые unit tests проверяют отсутствие public keyring
digest и отклонение symlink в userdata snapshot.
