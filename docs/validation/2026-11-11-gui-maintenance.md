# GUI maintenance Backup / Restore в новый путь

База: PR105, `b2e2e7e12d8d6175a3c9ec7ab29f4d1359ab6a19`, обычный merge.
Linux, Rust 1.85.1, `--locked`, jobs=1, общий rust-slot и flock собственного
transient-каталога. Evidence вне TTL:
`/home/myrs/projects/yonik/local/validation/p06-gui-maintenance/`.

GUI использует Qt FileDialog и отдельную maintenance-панель. Backup не принимает
пользовательский source. Restore создаёт новый файл; текущая БД не заменяется и
не переключается. Opaque MaintenanceAccess получает путь из открытого SQLite
соединения и хранит только путь и Zeroizing-ключ, без SQLite handles, Debug,
Serialize и key getter. Повторного unlock/keyring-запроса при копировании нет.
Копия требует того же существующего ключа; перенос ключа и recovery другого
устройства не реализованы. SQLite содержит ciphertext payload и структурные
метаданные, а не полное шифрование файла.

Admission допускает один запрос, проверяет request_id, ревизию, пути и занятость.
Активные подключения, диагностика, fetch и review получают Busy. Job отдельно
дожидается scheduler shutdown и освобождает настоящие service/routing handles.
Actor slots становятся None; supervisor обрабатывает отмену и Shutdown, а новые
команды получают Busy. Join единственного job связывает completion с запросом;
queued commands дополнительно проверяются по control generation. Instance lock
остаётся у supervisor до drain и reopen; при ошибке reopen состояние остаётся
offline до явного Shutdown. Если job не подтвердил drain (включая panic),
Closed не выдаётся и instance lock удерживается; требуется восстановление процесса. Успешный reopen выполняется под тем же instance lock.

SQLite admission ограничен пятью попытками с интервалом 10 мс. В copy job есть
30-секундный deadline, InterruptHandle и проверки между validation records.
Отмена публикации сериализована atomic CAS и не ждёт syscall hard_link.
Interrupt handle снимается до sqlite3_close, чтобы отмена не ждала внутренний
close mutex rusqlite. Один AEAD-вызов ограничен размером записи, но непрерываем.
Прерывание kernel I/O, fsync и жёсткий wall-clock предел завершения не обещаются.
После admission публикации Cancelled не подменяет результат. Ошибка cleanup или
fsync сохраняет Storage/неопределённость результата; UI предлагает проверить
выбранный файл и сохраняет ввод. Нижний HMAC recovery/ownership алгоритм PR105
не изменён.

Проверки кандидата (`candidate-*.log`):

- desktop lib: 39 passed, включая gated Stop/Shutdown, запрет Closed до release
  leaf и удержание directory lock до drain;
- actual DesktopHost под private D-Bus: 1 passed, Backup → Restore NEW → Shutdown;
- opaque access/cancellation: 2 passed; legacy storage_backup: 4 passed;
- backup fault/crash fixtures: 15 passed;
- cancellation admission, late publication и cleanup uncertainty: 3 passed;
- `clippy --all-targets -- -D warnings` для desktop/linux и fmt: passed;
- Qt 6.11.2 `/usr/lib/qt6/bin/qmltestrunner`: 5 passed (3 проверки плюс
  init/cleanup), offscreen, private D-Bus, Qt dialog fallback. Проверяется
  обработка clicked/accepted signals, выбранных путей, matching ACK и отмены;
  OS portal и взаимодействие мышью с системным file picker не проверялись.

Все runtime fixtures используют собственные пути и ephemeral ключи.
Пользовательские БД и Secret Service не затрагивались. Полная P06 и аппаратный
keyring/login-session gate не заявляются. PR остаётся draft для независимого
review supervisor/cancellation и оставшейся системной приёмки.
