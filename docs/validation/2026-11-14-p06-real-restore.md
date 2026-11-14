# P06: normal Backup → Restore NEW с настоящим private Secret Service

На accepted base `8ea826a04a745c8916e14a4329588c75f66b683e` выполнен один
focused normal Host lifecycle. Production, dependencies, GUI/backend/credentials,
старые A4/private copies не изменялись. Это закрывает конкретный пробел
R2ef705424, а не весь P06 или Linux acceptance.

Исторические GUI maintenance runtime fixtures использовали ephemeral key;
real Secret Service format4 GUI подтверждал backup/live DB reopen, но не
restored NEW. Missing/Locked из старого VM отчёта не повторялись.

## Actual наблюдения

Использован установленный KSecretD через сохранённый P02 --pam-login recipe,
без download/install. Private dbus-daemon config не содержит service directories.
Перед daemon имя отсутствовало; каждый inspect подтверждал owner PID=2029987,
UID=1000 и живой единственный управляемый foreground daemon. HOME, XDG и nonce
принадлежат свежему private каталогу 0700. Пользовательские keyring/БД не открывались.

1. Normal HostConfig создал отдельную live DB и один реальный master-key item.
   Через normal maintenance выполнен schema3 backup (request 100), затем
   import_text → review → approve_refresh сохранил synthetic профиль.
   Shutdown получил Closed ACK. Production offline подтвердил original,
   metadata, count=1, revision=1; наружу выведены только bool/count/revision.
2. Новый normal Host на live DB выполнил Backup (101): Complete/error=null,
   затем штатный Shutdown/Closed ACK.
3. Ещё один normal Host выполнил Restore (102) в первоначально отсутствующий
   `restored/subscriptions.sqlite`: Complete/error=null, Shutdown/Closed ACK.
   Restore не переключал live DB и не заменял существующий файл.
4. Missing-source negative (103) получил matching Failed/invalid_input и не
   опубликовал `negative/absent.sqlite`. После отказа source original/revision
   снова проверены; результат positive Restore не менялся.
5. Fresh normal Host отдельного процесса открыл restored data_dir: ready=true,
   sources_total=1, total=1, revision=1; Shutdown/Closed ACK. После его закрытия
   production credential API (create=false), repository и offline подтвердили
   original и metadata восстановленного профиля.

Между seed, backup, restore, negative, reopen и verify item оставался
`/org/freedesktop/secrets/collection/kdewallet/0`, count=1, unlocked, с теми же
Attributes. Inspector не вызывает GetSecret и не читает ключ. Decryption читает
ключ только через production subscription_master_key; injection/ephemeral/fallback
отсутствуют. SHA256 live DB после seed и всех последующих стадий совпал.
После остановки daemon имя исчезло; private bus завершён. Новых keys при reopen
не создано. Ключ, его hash, ciphertext и credential bytes в evidence не выводились.

GUI для этого proof не требовался: MaintenanceView.submit отправляет тот же
maintenance_start через bridge в DesktopHost. OS picker, physical login/PAM,
lock/unlock prompt и полный Linux/P06 scope этим запуском не проверяются.

## Binding и проверки

Перед cargo сохранены полный source archive и manifest 2132 файлов. Во время
build/clippy/runtime исходники и harness не редактировались; hashes до/после
build и runtime совпали побайтно. Build command, Rust 1.85.1 -Vv, exit0 и ELF hash
записаны **до execute**. Runtime исполнял сохранённую побайтную копию этого ELF.
Отчёт/evidence добавлены после завершения runtime и Clippy; они не входят в
замороженную версию исходников binary capsule.

Build --locked и focused Clippy --no-deps -- -D warnings PASS, jobs1, оба wrapper
пустые, shared rust slot и flock корня transient. Ruff check/format, rustfmt PASS.
Missing-source — actual negative с восстановлением/проверкой данных после отказа,
а не тест, повторяющий реализацию. Повторный runtime не потребовался.

[Публичные gates, sanitized events и hashes](../evidence/p06-real-restore-20261114/manifest.json).
Полные source archive/manifest, ELF, build/clippy/runtime logs вне TTL:
`/home/myrs/.t3/worktrees/yonik/t3-1c52ca16-f07156ea4067fb30/local/evidence/p06-real-restore-bound/`.
Прежний unbound ELF сохранён отдельно в `local/evidence/p06-real-restore/`,
**NEVER EXECUTED**, и не связан с новым source. Старый preflight остаётся
историческим NOT PASS checkpoint; данный отчёт добавляет actual proof.

После сохранения artifacts проверены terminal build/clippy sessions, завершённые
private daemon/bus и отсутствие ссылок собственного UID cwd/exe/fd/maps на target.
Под exclusive root flock `cargo clean` удалил 3974 файла / 1,9 GiB; собственный
transient root удалён. Сохранённые source/ELF/logs остались вне TTL.
