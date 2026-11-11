# P24 userdata: production Secret Service, upgrade и rollback

Используются реальные пакеты, уже принятые в #78/#119:

| Версия | Source | Package SHA-256 |
| --- | --- | --- |
| r20261110-2 | eec7a56b6b03f2b022a87682f9097ce78b61fa0b | e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0 |
| r20261111-1 | 6cf65a4cf9760a7df68b8eee10f012770b38604d | a10f0840737b2ababc572fbd432335abfd5c232c59ec0493aa0483d749abafcc |

Новых Rust/Go сборок нет, архивы не перепаковываются. Перед VM effects
`versioned-inputs.json` фиксирует source revision/archive hashes, PKGINFO,
BUILDINFO и archive ELF hashes. Installed ELF сверяется с соответствующим архивом.
Production custody/crypto/repository checks не изменялись; source freeze остаётся
6cf65a4. #107/#113 и другие новые app commits в эти бинарники не входят.

Стенд ориентируется на точный `fa8cd659432b928dbb683430273d0cbea74c3876`:
`tools/credential_vm_guest.py`, git blob `3853f2c6efae426126082996d44b1d43501db2ee`,
SHA-256 `fd14e3623550b52906bed3bb92127e59a267e89157c4492455f5583f72b81b69`.
Объект прочитан из git без изменений. Принятое evidence #118 находится в
`/home/myrs/projects/yonik/local/validation/credentiallife/acceptance.json`.
Его вспомогательные compiled examples здесь не используются: проверяется именно
`/usr/bin/yonik-desktop` из обоих пакетов, в обычном production mode.

OwnVM имеет отдельный builder HOME, private D-Bus без activation service dirs,
Xvfb без TCP listener, tray provider и foreground gnome-keyring с собственным
control directory. Пароль keyring генерируется внутри VM и передаётся через
stdin; raw master key не читает и не экспортирует fixture. Ключ создаёт сам old
production binary и затем загружает через обычный Secret Service custody path.
Evidence записывает owner PID/UID, unique owner и один unlocked item production
schema; fixture не вызывает GetSecret. После restart provider unique owner/PID
меняется, item сохраняется, новый процесс GUI расшифровывает прежний профиль.

Профиль `P24StableProfile` создаётся old GUI через loopback HTTP fixture с явным
«Локальный источник». Endpoint и fake credential не являются реальными данными.
Приложение само создаёт БД и encrypted profile payload. SQL используется только
для readonly проверки количества, ID и hash ciphertext; profile plaintext marker
в ciphertext отсутствует. Чтение проверяется отображением имени и hostname в
настоящем GUI, а не количеством файлов или фактом существования БД.

Old GUI production serializer создаёт preferences с language=en. New GUI читает
их и показывает English UI; затем через действительный language control записывает
новые поля `proxy_fetch_dns` и `subscription_interval_seconds`. После rollback
старый GUI всё ещё читает encrypted profile с тем же ключом, но settings parser
явно отклоняет future fields: показывает defaults/предупреждение и блокирует запись.
Попытка изменить язык показывает ошибку сохранения; исходные bytes preferences
остаются неизменными. Между фазами каталог БД/keyring/preferences не заменяется.

Снимки сравнивают SQLite bytes/UID/mode и encrypted payload, собственный fixture
keyfile, actual keyring file hashes/UID/mode и preferences. Fixture keyfile не
выдаётся за master key: фактическое использование ключа доказано повторной
расшифровкой persisted profile через production GUI. Перед/после startup helper
должен иметь строго inactive/3. Подключение engine и активация helper не выполняются.
При uninstall ELF исчезают, данные и production Secret Service item сохраняются.
Чужие nft table/dummy interface внутри VM сравниваются до/после всего маршрута.

Evidence: `/home/myrs/projects/yonik/local/validation/arch-production-119/`.
`run1` — исследовательский run с phase continuation: сохранены screenshots/logs
ошибочных способов закрытия окна (`windowclose`, затем `windowquit`); production
без tray намеренно не выходит по Close. Стенд использует настоящий tray Quit,
production behavior не меняется. Также сохранён OCR FAIL с латинским `He` вместо
кириллического `Не` в сообщении отказа сохранения. Screenshot показывает реальный
отказ, bytes preferences сохранены; нормализация OCR покрыта тестами.

`final` — отдельный автоматический run, остановившийся на timeout Arch mirror
при скачивании qt6-base. Pacman не обновил пакеты; FAIL manifest сохранён.
`final-retry` продолжает тот же overlay с единственным download retry, затем
пустым production HOME и полным автоматическим маршрутом. Исторические FAIL
логи/статусы не переписываются.

Signed base cache используется только read-only под BSD lock; pinned SHA-256
`794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003`.
Обновляются/перезагружаются только guest packages/kernel. Host packages, keyring,
сеть и настройки не меняются. Native Go/provenance gaps, full P25 и окончательная
Linux acceptance этим userdata gate не закрываются; final candidate review — root.

`final-retry` реально создал профиль и old preferences с пустого HOME, но
остановился перед upgrade: tray стартовал до готовности Xvfb. Его FAIL сохранён.
Добавлены readiness check display и проверка живого tray provider. `acceptance`
продолжил на том же overlay, production DB/keyring/password не удалялись и не
заменялись. Перемещён только каталог diagnostic logs прежнего FAIL.

В `acceptance` полный userdata route автоматически завершился PASS:

- old production GUI повторно прочитал зашифрованный профиль;
- gnome-keyring provider перезапущен, unique owner/PID изменился, один unlocked
  production item сохранился;
- new GUI прочитал тот же ciphertext profile и old preferences, затем настоящим
  serializer произвёл future preferences;
- rollback old GUI прочитал encrypted profile, явно предупредил о неизвестных
  preferences, отказал записи и сохранил bytes;
- uninstall сохранил БД/profile/keyring/fixture keyfile/preferences, удалив ELF.

Raw outer status `acceptance/versioned-run.json` остаётся FAIL: только после
успешного userdata route root-wrapper увидел изменение MAC чужого dummy interface.
Ifindex не менялся; ранний baseline захватил MAC до завершения udev. Неизменённые
before/after и raw FAIL сохранены. Добавлен `udevadm settle` перед baseline.
Целевая `foreign-settled` перепроверка old→new→old→uninstall после settle — PASS:
все interface attributes/nft table и production data hashes/UID/mode сохранены.
Повторять доказанный crypto GUI route для этого исправления не потребовалось.

Итоговый `acceptance.json` в корне evidence объединяет независимые фактические
результаты, не переписывая raw FAIL runs. Ciphertext profile: ID=1, 105 bytes,
SHA-256 `dbe8329be286360be55c8ca573feb10138cd9c35c9cd37c187ecdc979ff7834a`.
Hash payload одинаков на всех фазах; DB bytes, fixture keyfile и actual keyring
file metadata также одинаковы. Old preferences сохраняются при upgrade до
намеренной записи new producer; future preferences неизменны при rollback и
uninstall. Проверено наличие English UI после чтения old language=en новым GUI.

Actual guest route SHA-256:
`da4bc3506b38082845972eedd8ea135d3255ce7cd98397b40b932dbf5a6ca151`;
bytes `production-route.py` из artifacts совпали с final source. Wrapper отличается
от проверенного userdata run одной строкой settle, проверенной отдельным actual
foreign-resource test. Четырнадцать Arch tests, Ruff и diff check проходят.
Обе собственные VM выключены с exit0, собственные overlay/cloud-init/password/SSH
key удалены. Public signed base cache сохранён с исходным SHA-256. Никаких
Cargo targets в этом продолжении не создавалось.
