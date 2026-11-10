# Адреса, маршруты и конфигурация системного TUN

Срез P12/P13 продолжает [владение TUN](helper-owned-tun.md). Полная системная
сессия service/controller/GUI, DNS и production RPDB пока не подключены.

## Дочерние ресурсы

`Resource::Address` хранит tun_id, фактический ifindex, IP и prefix. Документ
журнала требует существующего родительского TUN. Допустимы native IPv4/IPv6
unicast адреса с ненулевым prefix; unspecified/loopback/multicast/link-local,
IPv4 broadcast и IPv4-mapped/compatible IPv6 отвергаются. Это интерфейсные
адреса, а не канонические CIDR-маски маршрутов.

`TunExecutor::configure_link` проверяет полную метку TUN и применяет MTU
1280…9000/up через закрытый Netlink action. Адреса добавляются с NOPREFIXROUTE
и NODAD: обычные main connected-prefix routes не появляются неявно. Поля,
которые kernel нормализует по семейству, сверяются явно. IPv6 kernel LL
наблюдается отдельно и не выдаётся за вручную назначенный глобальный адрес.

`TunRoute` требует canonical prefix, table 256…u32::MAX-1 и ненулевой priority;
protocol фиксирован 253, ifindex берётся у проверенного TUN. Таблица не свободна,
если содержит неизвестный маршрут или на неё ссылается существующее RPDB rule.
Наличие только совпавшего protocol/table не доказывает владение. До появления
собственного rule executor не принимается даже чужая ссылка на пустую таблицу.

Перед CREATE|EXCL записывается intent, затем требуются kernel ACK и точный dump.
Свежая запись с совпавшим ранее существовавшим объектом не присваивается.
Existing Intent после reopen остаётся unknown. Confirmed+Absent требует явного
cleanup отсутствующего ресурса и нового intent; неизвестный повтор не получает
старое durable подтверждение. Confirmed+exact использует полную TUN-метку как
anchor в модели доверенных CAP_NET_ADMIN actors.

## Очистка и внешние изменения

Cleanup идёт в обратном порядке, адреса/маршруты — до родителя. Если ребёнок не
удалён, callback родителя не запускается. Ошибка сохраняет остатки и необходимость
новой сверки. Удаление адреса или маршрута требует точного match всех selectors,
flags и допустимых metadata; broad flush/replace отсутствуют.

Перед DELLINK проверяются не только journal children, но и actual topology.
Неизвестные адреса, routes/multipath/nexthops и interface-bound RPDB rules на
этом ifindex дают Foreign. Чужие ресурсы не исчезают вместе с нашим интерфейсом.
Kernel-derived исключения ограничены IPv6 LL с IFA_PROTO=3, точными scope/flags/
cache metadata и производными fe80 prefix/local/multicast routes. Неизвестное
представление даёт отказ, broad `proto=kernel` не считается доказательством.

Для Changed/Interrupted snapshot выполняются максимум три read-only попытки с
общим сроком 5 секунд. Записи не повторяются вслепую; отмена сохраняет lease до
завершения kernel ACK/child reap. После snapshot повторно проверяется namespace,
parent identity и actual ресурс. Полная конкурентная транзакция с другим root
не обещается.

## Компилятор

`compile_system` и `compile_system_policy` добавляют derived TUN name/MTU и
ненулевую socket mark, сохраняя outbound credentials и loopback probe.
Policy-вариант принимает только IpOnly; Names/Fake DNS требуют отдельного
подтверждённого resolver/provenance пути и не включаются этим API.

Xray получает пустой массив autoSystemRoutingTable, выключенный
AutoSystemDnsToGateway и не получает gateway; адресами/маршрутами владеет helper.
Для sing-box передаются те же адреса, что назначил helper, auto_route/auto_redirect
выключены, dns_mode disabled и external_configuration включён. Host до Apply
должен проверить бинарник, его возможности, endpoint resolution и escape path.
Само получение JSON не подтверждает ни applied, ни прохождение DNS gates.

В stock sing-box 1.14.4 `auto_route:false` всё равно создаёт IPv6 default route и
OIF-rule в случайной таблице. Нужен минимальный pinned adapter, экспортирующий
существующий EXP_ExternalConfiguration sing-tun: он передаёт управление адресами,
routes/rules helper. Штатные MTU/up/offload операции библиотеки остаются, поэтому
host сверяет ownership/ifindex и ожидаемые настройки до и после старта; минимальные
права worker ещё требуют отдельного gate. Обычный бинарник обязан отклонить неизвестное поле при
config-check; это проверяется до сетевых эффектов. [Upstream sing-tun](https://raw.githubusercontent.com/SagerNet/sing-tun/040db71fcd33/tun_linux.go).

CLI-флаги --tun-id/--tun-mtu/--routing-mark/--tun-address служат инструментам
разработчика; обычный запуск GUI от них не зависит. Секретный JSON выдаётся
только с явным --emit-secret-config. Сводка остаётся compiled_not_checked.

## Приёмка

Синтетический тест использует actual TunExecutor, скомпилированный VLESS профиль,
отдельный VLESS server и echo. Проверяются две семьи и TCP/UDP, сохранение
глобальных адресов/NOPREFIXROUTE, отсутствие лишних main routes, отрицательный
контроль после остановки peer и обратный cleanup. Fixture-RPDB не выдаётся за
production rule executor. [Evidence](../validation/2026-11-10-system-tun-configuration.md).
