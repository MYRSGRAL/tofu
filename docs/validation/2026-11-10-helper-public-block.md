# Публичная блокировка: правила и реальные пакеты

Продолжение P12 поверх nft executor PR #32. Общая готовность P12 и системного
VPN не заявляется: service/IPC, durable policy payload и topology binding ещё
нужны. [Контракт](../specs/helper-public-block.md).

## Реализация

- Типизированная ограниченная политика IPv4/IPv6: точечные transport/bootstrap
  исключения с UID/mark/интерфейсом, private VPN guards, явные LAN и TUN.
- Отдельные RS/NS/NA на выбранных link interfaces, hoplimit=255/code=0.
- Создание и замена полной таблицы с правилами в одной nft transaction.
  Owner marker связывает ресурсы и digest политики. Подтверждение требует
  полного совпадения фактической chain/rules, а не только имени и comment.
- Durable update intent и сброс прежнего ephemeral подтверждения до новой
  async сверки. Ошибка или отмена не сохраняют прежнее applied evidence.
- Сессионный cleanup не вызывает Unlock. Явная замена политики после Stop
  убирает session guards, сохраняя публичную блокировку; явный Unlock снимает
  только собственную таблицу.

## Проверки

Rust 1.85.1, существующие зависимости:

- Workspace: **417 passed, 0 failed, 9 ignored**, 42 suites.
- Managed network unit suite: **23 passed, 2 ignored**.
- Отдельно real namespace traffic test: **1 passed**.
- Workspace/all-targets Clippy `-D warnings`, fmt и diff check: PASS.
- Python: **100 passed**; Ruff format/check всех tools: PASS.

Real traffic fixture создаёт только собственные user/net/mount/pid namespaces
с отдельными proc, sysfs, `/tmp` и `/run`. Два veth-пути ведут к echo peer:
синтетические публичные, LAN и private IPv4/IPv6, TCP/UDP, два порта.
Namespace guard проверяется до эффектов; тестовый cfg(test) constructor не
меняет production trust-check executable. Сеть пользователя не менялась.

Проверены:

- Доступность до защиты; публичные TCP/UDP обоих семейств блокируются.
- LAN и конкретный VPN разрешены; public transport/bootstrap требуют точных
  mark/порта/назначения. Неверная mark, порт и соседний IP не проходят.
- Перенос private маршрутов на LAN не создаёт fallback. После снятия session
  guards приватные назначения вновь обслуживаются ОС, публичные остаются
  заблокированы. После явного Unlock обычная сеть доступна.
- Session-table cleanup, Drop/reopen executor и journal сохраняют public block.
- Конфликт exclusive create откатывает также chain/rules; чужая таблица
  сохраняется. Чужие DNAT-таблицы остаются после Unlock.
- NDP baseline без firewall, отрицательный контроль без ND-правил,
  положительный контроль после атомарной замены и холодный cache с обеих сторон.
- Ошибка повторной сверки после успешного Apply и отмена на ожидающем gate
  оставляют reconciliation обязательной.

## Review и исправления

Независимое review нашло P2: повторная сверка могла сохранить старое
`reconciled` после ошибки/отмены. Исправлено в `0a90a8e`; узкий review закрыл P2.
Правка добавлена также в PR #32 до его слияния.

Проверка cold NDP выявила одновременно ограничение политики и DAD-гонку
стенда. Fixture теперь задаёт уникальные link-local адреса с nodad; итоговый
тест требует корректный baseline, отказ без ND и успех с явными ND-правилами.
Новые правила и semantic normalization прошли узкое независимое review.

Дополнительный runtime RED воспроизвёл обход LAN→public через существующий
output DNAT: priority -150 проверял назначение до переназначения на -100.
Priority 0 исправляет порядок; GREEN проверяет IPv4 и IPv6 и сохранность
чужого NAT. Узкое review этого исправления без новых findings.

При общем Python-прогоне выявлен flaky benchmark-тест: ACK stdout не означает
полное чтение независимого stderr pipe. `d479217` добавляет bounded event-driven
drain в тесте; assertions объёма и лимита файла сохранены. Production benchmark
не менялся. Независимое review без findings; 17 benchmark tests и все 100 Python
tests после исправления прошли.

## Остаток

Это library-level output guard, не полная гарантия Linux VPN. Не закрыты
service/IPC/пользовательская авторизация, durable desired/applied policy storage,
смена topology/переиспользование ifindex, TUN, DHCP/local discovery adapters,
G-DNS/G-APP, полный Stop/recovery и forwarded/container traffic. Настройка GUI
пока не включает эти правила на host. Реальные namespace-тесты не заменяют
проверку обоих engine и всей системной матрицы.

Evidence: `local/validation/helper-public-block/`; own build после проверок
очищается штатным cargo clean. Для повторения ignored test нужна новая сборка
и runner с перечисленной изоляцией, а не запуск в host netns.
