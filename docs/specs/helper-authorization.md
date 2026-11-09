# Авторизация системного network helper

Контракт реализуемого компонента `tofu_linux::helper`; основание —
[границы Linux](platforms.md#2-linux-границы-доверия) и
[план helper](../superpowers/plans/2026-11-09-linux-network-helper.md).
Компонент не запускает engine, не применяет сеть и не устанавливает системный сервис.

## Вход и доверие

Доверенный обработчик D-Bus передаёт настоящий входящий `Message`, фиксированное
`NetworkAction`, режим `Prompt` и `Cancellation`. Поле UID из payload не является
источником идентичности. Отправитель определяется по уникальному bus name из
заголовка; UID запрашивается у bus daemon. Header, самостоятельно построенный
внутри процесса, не заменяет сообщение от доверенного D-Bus dispatcher.

`PeerAuthorizer::system` подключается к каноническому system-bus socket
`/run/dbus/system_bus_socket`; session bus и пользовательский адрес из environment
не используются как production fallback. Владелец `org.freedesktop.PolicyKit1`
разрешается до запроса и обязан принадлежать UID0. Запрос отправляется его
зафиксированному unique name.

Доступны только действия `cloud.highpriority.tofu.manage-network` и
`cloud.highpriority.tofu.unlock-network`. Polkit subject — `system-bus-name`
с именем настоящего отправителя. Details пусты: клиент не задаёт shell/path,
сообщение диалога, действие polkit либо чужой subject.

## Ответ и повторная сверка

Raw replies от bus daemon требуют sender `org.freedesktop.DBus`; ответ проверки
требует pinned unique name authority. Совпадения reply serial недостаточно для
решения о разрешении. Размер и тип ответа проверяются до использования полей.
Неизвестная/повреждённая структура, неверный sender, конфликтный результат,
недоступность службы и ошибка протокола не дают разрешения.

После положительного ответа повторно проверяются живой peer/UID и текущий
владелец authority. Исчезновение peer или смена authority отвергает результат.
`PeerIdentity` содержит UID и unique sender для текущего успешного запроса;
это не сохраняемый permission token и не гарантия отсутствия будущего revoke.

Prepare и Apply должны выполнять собственную свежую проверку. Сохранённый
PeerIdentity не позволяет обойти проверки окружения/digest/generation из
PreparedPlan. Не допускается трактовать доступность system bus либо успешный
прошлый запрос как постоянное право на сетевые операции.

## Отмена и ограничения

Одновременно выполняется не более четырёх проверок. До начала проверки
обрабатываются отмена и отсутствие свободного слота. Закрытие источника
Cancellation тоже отменяет операцию.

Непосредственная проверка без интерактивного диалога ограничена пятью секундами;
явно разрешённая доверенным caller интерактивная проверка — 120 секундами.
Подготовительные IPC-вызовы тоже ограничены. Prompt Allow допустим для действия,
инициированного пользователем; фоновые проверки используют Never.

Для отмены генерируется собственный уникальный cancellation ID. Timeout/отмена
запрашивают CancelCheckAuthorization у того же pinned authority с отдельным
коротким deadline 250 мс. Поздний положительный ответ не становится успехом.
Удалённая отмена при недоступной службе может остаться неподтверждённой; это
не разрешает локальному caller выполнять эффект.

Входящая body ограничена 64 KiB, ответы проверок — 4096 байт. Это пределы
данного RPC-компонента после получения Message библиотекой D-Bus, не заявление
о предотвращении всех транспортных аллокаций или лимит количества профилей.
Ошибки представлены enum без raw D-Bus error strings. Polkit details и temporary
authorization IDs не возвращаются и не записываются в логи.

## Приёмка компонента

Тесты выполняются на отдельном dbus-daemon и fake authority. Host system bus,
polkit пользователя, диалоги и сеть не затрагиваются. Нужны реальные round trips
allow/deny/challenge, фиксированные subject/action/flags, исчезновение peer,
смена/недоверенный владелец authority, malformed/oversized/forged reply,
отмена/timeout/поздний allow, уникальные cancellation IDs и независимость
параллельных проверок. Тестовый constructor/config не экспортируется в production API.

Основные первичные источники: установленный официальный polkit manual
`/usr/share/gtk-doc/html/polkit-1/eggdbus-interface-org.freedesktop.PolicyKit1.Authority.html`,
[публичный контракт polkit](https://www.freedesktop.org/software/polkit/docs/latest/eggdbus-interface-org.freedesktop.PolicyKit1.Authority.html),
[D-Bus specification](https://dbus.freedesktop.org/doc/dbus-specification.html)
и исходники закреплённого zbus4.4.0. Сам компонент не заменяет изолированную
приёмку сервиса/политик установки и полного NetworkHost.
