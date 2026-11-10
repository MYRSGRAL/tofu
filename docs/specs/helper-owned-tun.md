# Владение persistent TUN

Продолжение P12/P13: `managed_network::tun::TunExecutor` создаёт и удаляет
типизированный persistent single-queue TUN. Полная системная сессия пока не
подключена к этому исполнителю; GUI продолжает использовать local SOCKS.

## Создание и подтверждение

Вход — проверенный Ownership, record ID и numeric name_id. Имя строится как
`tofu` + 8 hex-цифр для совместимости. До эффекта записывается Session intent;
внешний клиент не передаёт имя, alias, команду, путь или ifindex.

Исполнитель закрепляет FD сетевого namespace и доверенного `/usr/bin/ip`:
root ownership, отсутствие group/world write, NOFOLLOW на каждом компоненте,
обычный executable без setuid/setgid и дополнительных hard links. Environment
очищен; shell не используется. Аргументы фиксированы: `tuntap add dev <derived>
mode tun user <owner_uid>`. Single-queue не означает опцию `one_queue`: это
отсутствие `multi_queue`, в соответствии с реально проверенным режимом ядер.

`ip tuntap add` использует IFF_TUN_EXCL; создание поверх существующего TUN
отклоняется. Это проверено в namespace повторным вызовом и сохранением исходной
идентичности. [Исходник iproute2](https://raw.githubusercontent.com/iproute2/iproute2/main/ip/iptuntap.c).
Поставка должна сохранять этот контракт системного iproute2.

После успешного создания helper наблюдает kind=tun, ARPHRD_NONE и actual ifindex,
затем RTM_SETLINK по индексу записывает полную метку: версия, UID/session/generation,
netns dev/ino и record ID. Подтверждение требует kernel ACK и повторного dump
с совпадающим index/name/kind/alias. IFLA_IFALIAS читается ограниченно, его Debug
не раскрывает. Неизвестные flagged-представления остаются opaque.

## Повторы, ошибки и cleanup

Совпадение имени без полной метки никогда не считается владением. После сбоя
между create и alias остаются неизвестный интерфейс и durable intent: повтор
не присваивает и не удаляет его автоматически. Помеченный собственный TUN
после повторного открытия журнала сверяется заново.

Cleanup требует Session record и подтверждённый ресурс. Намерение удаления
сохраняется до RTM_DELLINK по actual ifindex; удаление по одному имени не
используется. ACK дополняется проверкой отсутствия. Ошибка сохраняет остаток.
Если полная метка обнаружена под другим именем, возвращается Evidence и запись
остаётся: отсутствие прежнего имени не означает успешную очистку. Автоматический
ремонт переименованной сессии пока не реализован.

Общий process-wide lease сериализует TunExecutor. Отмена ip future передаёт
kill/wait и lease отдельной задаче; следующая операция ждёт reap. Переданный
Netlink запрос также удерживает lease до ACK/deadline после Drop caller.
Время — 5 секунд, stdout/stderr ip — по 256 KiB, Netlink ответ — 64 KiB.
ACK принимается только от kernel, с точными seq/port и исходным заголовком.
Никаких автоматических повторов неизвестного эффекта.

Другой доверенный CAP_NET_ADMIN может менять ресурсы между check/write; alias
не является криптографической защитой от root. Завершение kernel-операции после
таймаута не доказывается одним process fence. Неопределённость не превращается
в applied/cleanup-success.

## Проверки и остаток

Оба закреплённых ядра и адаптированный sing-box используют заранее созданный
single-queue TUN: TCP/UDP IPv4/IPv6, рост RX/TX, TERM/KILL, сохранение ifindex и
alias; контроль без ядра не проходит. Multi-queue несовместим в проверенных
конфигурациях. [Evidence](../validation/2026-11-10-owned-tun.md).

Остаются адреса/routes/rules/resolved, привязка registry worker/PreparedPlan,
service/controller/GUI, компенсация всей сессии и полная crash/network-change
матрица. Executor не подтверждает прохождение G-LINUX/G-DNS/G-APP и не закрывает P12/P13.
