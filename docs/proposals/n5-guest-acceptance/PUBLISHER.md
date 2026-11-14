# Закрытый source publisher N5

`publish_captures.py` добавляет только публикацию результатов существующих
коллекторов. Импорт не выполняет I/O, CLI отсутствует. Публичные операции:

- `publish_capture(stage, nonce, case) -> None`, stage только baseline/connected/stopped;
- `publish_unit_closure(nonce, case) -> None`.

`None` означает завершение записи и fsync файла и каталога, но **не PASS**.
Ошибки вызывают `PublicationError`. Данные ACTUAL остаются наблюдениями.
Publisher не выдаёт grant, не подтверждает acceptance, native authority или
production readiness. Не принимает path, argv, env, blob, backend или executor.

## Gate и данные

До создания временных файлов и вызова коллектора проверяется существующий
root UID + `_authorize`: ROOT_CAPTURE_AUTH_V1 / USER_GRANTED_GUEST_CAPTURE,
nonce/case/stages, sealed input и его digest, source и manifest digest.
Создание authorization/input не реализовано и не разрешено этим модулем.
Используются существующие sealed-read helpers; gate и байты collector source
повторно сверяются перед записью. Оба коллектора также выполняют собственный gate.
Схема проверяется с точными наборами полей; provenance связывается с source,
manifest, case, stage, nonce, authorization digest и digest установленного
`collect_observations.py`. Unit closure дополнительно проверяет точные свойства
units/helper и digest trusted systemctl. UNAVAILABLE и невалидные результаты
не публикуются. Контракт существующего unit collector именует только общий
collector digest: digest самого unit_closure_capture.py в схему не добавляется.

Это source-bound контракт, а не криптографическая аттестация Python interpreter
или imported module. Нужны доверенные установка, root и окружение импорта.
Manifest hash связывает контекст, но publisher не перепроверяет содержимое
manifest и не заменяет runtime-проверки Rust consumer (canonical units, live
manager, helper ELF against manifest). Против вредоносного root защиты нет.

## Файловая публикация

Каталог уже должен существовать: `/run/yonik-acceptance`. Вся цепочка от `/`
открывается через O_DIRECTORY/O_NOFOLLOW и проверяется на root ownership и
отсутствие group/other write. Ни mkdir, ни обход symlink не разрешены.
Dirfd удерживается; повторный обход проверяет dev/inode назначения перед
публикацией и после неё. Операции с именами выполняются относительно dirfd.

Имена только `capture-baseline.json`, `capture-connected.json`,
`capture-stopped.json`, `unit-closure.json`. Любое существующее имя, включая
symlink/hardlink и capture того же case/nonce, отвергается до сбора данных.
Повторное exclusive связывание закрывает конкурентную публикацию.
JSON ограничен 1 MiB, без nonfinite. Временное непредсказуемое имя создаётся
O_EXCL/O_NOFOLLOW с 0600; проверяется regular/root/single-link inode.
Полная запись учитывает short writes, затем fchmod 0444 и fsync файла;
exclusive hard link создаёт конечное имя, временная ссылка удаляется,
проверяется single-link, выполняется fsync каталога. Rename с overwrite нет.

До связывания конечного имени ошибка не создаёт success-файл. После связывания
ошибка, включая fsync каталога или смену path identity, означает
`PublicationUnknown`: уже связанный собственный capture сохраняется, durability
**не подтверждена**. Его наличие нельзя интерпретировать как успешный возврат.
Не повторять публикацию автоматически; требуется внешний разбор/очистка в
отдельно авторизованном lifecycle. При аварии процесса может остаться temp или
capture; publisher не выполняет массовую очистку и не удаляет старые captures.
Окна между системными вызовами не защищают от root, способного менять trusted
каталоги; неподконтрольный root исключён моделью доверия.

## Обязательный внешний координатор — отсутствует

Publisher не знает фаз driver и не может их выдумывать. Координатор обязан
предоставить аутентифицированный запрос с точными `stage/nonce/case`, ожидаемой
фазой driver и monotonic deadline. До запуска он должен проверить соответствие
запроса текущей фазе; ожидать завершения публикации до deadline и подтвердить
его driver только после успешного возврата. Timeout/unknown запрещает PASS и
автоматическое повторение. Unit closure также требует явной точки запроса и
срока; он не выводится из имени stage. Нужны сериализация lifecycle, fresh root
и правила удаления старых captures. Этот интерфейс здесь описан, но транспорт,
планировщик, cancellation/deadline enforcement не реализованы.

Существующие collectors имеют собственные ограниченные чтения/команды;
publisher не добавляет observers/RPC, не управляет service, VM, сетью или driver.
Нет общей гарантии wall-clock deadline на весь сбор или fsync. Нельзя выдавать
успешный capture после просроченного запроса за acceptance нужной фазы.

## Проверки

`python -m unittest discover -s docs/proposals/n5-guest-acceptance -p test_publish_captures.py`
и та же команда с `python -O`. Тесты используют только mocks и собственный
TemporaryDirectory: реальные Popen, proc, DBus, systemctl, сеть и VM запрещены.
Реальные root-owned /run captures и grants тесты не создают.
