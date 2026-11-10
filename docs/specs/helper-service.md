# Версионный helper service

Срез P12: отдельный `yonik-helper`, canonical system D-Bus и доверенный host
для durable public-protection. Это режим явной блокировки публичного трафика;
он ещё не является VPN-сессией и не подключён к GUI.

## Закрытый IPC v1

Bus/interface: `cloud.highpriority.yonik.Network1`.
Object: `/cloud/highpriority/yonik/Network`. Каждый метод принимает и возвращает
одну JSON-строку, application body ограничен 64 KiB. DTO запрещают неизвестные
поля. Отправитель и UID берутся из входящего D-Bus сообщения и credentials daemon;
UID, пути, ifindex, argv, policy JSON и сетевые команды клиент не передаёт.

- `Prepare`: `{version:1,mode:"public_protection",allow_lan:false}` возвращает
  одноразовый handle после свежей `manage-network` авторизации. Host читает
  topology, выполняет `nft -c` без применения и привязывает artifact к поколению.
- `Apply`: `{version:1,handle:[32 bytes]}` заново авторизует того же peer,
  проверяет context/digest/TTL и применяет durable policy. Попытка одноразовая.
- `Stop`: `{version:1}` заново проверяет `manage-network`, инвалидирует старые
  artifacts и сохраняет явно включённую публичную блокировку.
- `Unlock`: `{version:1}` требует отдельную свежую `unlock-network` авторизацию;
  снимает только собственные подтверждённые правила через durable intent.
- `State`: `{version:1}` сверяет фактические правила с журналом владельца.
- `Capabilities`: `{version:1}` сообщает `public_protection:true`,
  `system_tun:false`, `allow_lan:false`.

Состояния: `no_managed_policy`, `verified_rules`, `recovery_required`, `unknown`.
`verified_rules` означает совпадение правил и актуальность bindings текущего
процесса; оно не подтверждает полноценную TUN/DNS-сессию. После перезапуска
найденные старые правила дают `recovery_required`, а не ложное подключение.

LAN временно отклоняется до авторизации: без live-reconciler старый разрешающий
prefix/ifindex мог бы остаться после смены сети. Deny-only разрешает loopback и
ограниченный link-local NDP; обычный TCP/UDP по IPv4/IPv6 блокируется.

## Сериализация и хранение

Не более четырёх обслуживаемых RPC одновременно; подготовленные artifacts
используют существующие общие лимиты и TTL. Ожидание интерактивного polkit не
удерживает mutex host. Apply захватывает его после свежей авторизации, в callback
проверки context, и сохраняет до завершения эффекта. State отвечает Busy при
реальном занятом host. Отсоединение peer удаляет его handles.

Файловый I/O/fsync и nft исполняются в blocking-задаче со своим runtime. Отмена
вызывающего future не отпускает backend mutex до окончания durable перехода и
reap процесса. Executor дополнительно сериализует nft в пределах процесса.

Host использует только `/run/yonik/network`; владельца восстанавливает из
защищённого журнала. Смена UID допустима лишь для пустого журнала без ресурсов и
политики. Namespace журнала должен совпасть с текущим. Случайный process seed и
монотонный authorization epoch инвалидируют контекст при изменении состояния.
Legacy polkit IDs `cloud.highpriority.tofu.manage-network` и
`cloud.highpriority.tofu.unlock-network` сохраняются для совместимости.

## Межпроцессный lifecycle

Файлы `packaging/` описывают root systemd unit, D-Bus activation/config и polkit.
Сервис требует unified cgroup v2. `ExecStartPre` и entrypoint до эффектов проверяют
фиксированный cgroup через FD-relative NOFOLLOW: никаких чужих живых PID в своей
группе и потомках. Чтения, обход и глубина ограничены; неизвестность даёт отказ.
`KillMode=control-group`, bounded Stop и SIGKILL убирают прежних workers до нового
старта. Проверка не доказывает завершение всех операций ядра или reap zombie.

RuntimeDirectory сохраняется и после Stop: иначе journal мог бы исчезнуть при
оставшейся в ядре блокировке. Права 0700/0600 и root ownership обязательны. Unit
ограничивает capabilities до NET_ADMIN, файловую систему, устройства, namespaces
и core dumps. Реальная установка/обновление unit проверяется отдельно в P24;
эта работа не устанавливает и не включает системный сервис на машине пользователя.

## Остаток P12/P13/P16

Нужны системный клиент и GUI, profile/engine PreparedPlan, worker lifecycle и TUN
обоих ядер, DNS/routes/resolver, cleanup всей сессии, live topology reconciliation
и безопасный LAN/VPN. Полный system-service integration и install/uninstall
остаются отдельными критериями. [План](../PLAN.md),
[evidence текущего среза](../validation/2026-11-10-helper-service.md).
