# Наблюдение сети и свежесть TCP/HTTPS-проверок

## Реализованный контракт

Linux host получает события RTNETLINK (link/address/route/rule/nexthop) и системного
D-Bus (NetworkManager/systemd-resolved). Нет таймеров опроса, автоматических
проверок серверов или переподключений. Watch объединяет ожидающие изменения до
последнего снимка. Поколение включает случайный ID наблюдателя и счётчик без
переполнения; это correlation token, а не разрешение менять сеть.

Смена поколения удаляет старый TCP-результат и отменяет выполняющиеся и ожидающие
TCP-команды. Результат дополнительно проверяется при приёме. HTTPS-проверка
привязана к тому же окружению: прежний успех больше не даёт `Connected`, сессия
остаётся в `Limited`, ядро продолжает работать. Повтор — явное действие пользователя.
GUI показывает доступность трёх источников наблюдения и предупреждает о неполном
покрытии. В DTO есть поколение, но в интерфейсе его технический ID не выводится.

## Проверенные сценарии

- Парсеры: повреждённые/truncated сообщения, kernel sender, смена адреса/MTU/
  gateway/правил, статистика и lifetime, ограничение размера кэша, повтор NEW.
- Частный `dbus-daemon`: свойства и invalidated properties, поддельный отправитель,
  смена/потеря/возвращение владельца службы, повреждение сообщения, отключение
  транспорта, отмена во время запуска. Системные службы не менялись.
- Реальный RTNETLINK в новом user/network/PID/mount namespace: **23 изменения**,
  включая IPv4/IPv6, nexthop и два rules одинакового priority с обоими удалениями.
  Пропусков нет; в idle stamp стабилен. SIGINT/SIGTERM подтверждают `closed` и reap.
- TCP backend в отдельном namespace: очистка готового результата, отмена inflight,
  новая ручная проверка в новом поколении, отсутствие автопроверок, cleanup.
  Fail-closed guards отвергают host namespace, отсутствующий маркер и чужие интерфейсы.
- Детерминированные unit-тесты: запоздалый успешный callback старого поколения,
  invalidation старого proof и команда TCP, уже находящаяся в ограниченной очереди.
- Qt offscreen с Xray 26.10.31 и sing-box 1.14.4: реальный HTTPS 204 и TCP,
  затем route event → `Limited/network_changed`, очистка обоих результатов.
  Тот же PID и starttime sealed-memfd ядра остаются живы до Stop; затем
  `Disconnected → closed`. Снимок снимается самим окном, без desktop capture.
- Ещё два Qt-сценария обоих ядер меняют маршрут во время `Checking`: собственный
  HTTPS-handler уже принял запрос, но удерживает 204 через Event. После `Limited`
  разрешается поздняя попытка ответа: успешный flush и закрытый клиентом транспорт
  записываются раздельно. До Stop требуется terminal исход и новый snapshot
  revision 2 через Refresh собственного HTTP-fixture: он ждёт завершения HTTPS.
  Задержка ответа 800 мс больше screenshot dwell; таймер не служит барьером.
  `Connected` не появляется ни разу. Ядро живо, Stop штатный.
  Все четыре сценария повторены на release-бинарнике после исправления очереди.

Независимые ревью нашли три P2 и проверили их исправления: подавление второго
DEL при одинаковом priority (`5ff93b2`) и сохранение старой TCP-команды в очереди
(`a0167cc`), а также отсутствие барьера перед Stop в тесте позднего HTTPS (`1feb369`).
Первый дефект также воспроизведён настоящим kernel event до исправления.
Успешный flush доказывает отправку сервером, но не выполнение callback клиентом;
защита от уже сформированного позднего callback проверена отдельным unit-тестом.

## Воспроизведение

Используйте собственный transient build-каталог и удерживайте `flock` его корня,
как в README. После сборки выполните:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build -p tofu-linux --example network_watch --locked
python tools/network_watch_smoke.py "$CARGO_TARGET_DIR/debug/examples/network_watch" --evidence /путь/к/evidence
python tools/desktop_smoke.py "$CARGO_TARGET_DIR/release/tofu-desktop" --case network --engine-dir /путь/к/engines --output /путь/к/gui-evidence
```

Оба Python native gate сами создают изоляцию и проверяют её до изменения сети.
Для ignored backend-теста `route_changes_invalidate_completed_and_inflight_tcp_checks`
требуется такой же новый namespace и переменная `TOFU_HOST_NETWORK_NS` с inode
исходного host namespace; прямой запуск не разрешает мутаций. Local evidence
хранится в `local/validation/network-observation/`: `native-fixed`, `backend`,
`https`, `release-network-barrier`; исходный отрицательный результат — `native/failure-report.json`.
Бинарники и private fixture не добавляются в Git.

## Границы доказательств

Это наблюдение с момента подписки, без initial netlink dump и атомарного снимка.
Оно не закрывает NetworkHost Prepare/Apply, G-DNS, TUN, DNS-pinning, Fake-IP epochs,
роутинг приложений или восстановление. Portable DnsEpoch не заменён этим stamp.

При переполнении/повреждении netlink задача завершает наблюдение и снимает
покрытие. Кэш ограничен 4096 ключами и 4 MiB, datagram — 64 KiB. В D-Bus очереди
ограничены (64 owner/128 property), body проверяется перед deserialization (256 KiB).
Потеря всей шины требует перезапуска наблюдателя; перезапуск отдельной службы
отслеживается по смене owner без опроса. Недоступный источник не считается покрытым.

Offscreen не доказывает живую приёмку KDE Wayland, трея, HiDPI или клавиатуры.
GitHub Actions ранее останавливался до шагов из-за billing lock аккаунта;
локальные проверки и независимое ревью не заменяют зелёный CI после снятия блокировки.
