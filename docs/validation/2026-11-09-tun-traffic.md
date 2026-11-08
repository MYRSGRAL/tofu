# TCP/UDP IPv4/IPv6 через TUN: 9 ноября 2026

## Метод

`tools/engine_traffic.py` запускает отдельный user/network namespace без sudo.
При входе проверяются отличный от хоста inode namespace и единственный
интерфейс lo. Доступа к внешней сети у стенда нет; DNS и сеть хоста не меняются.

Внутри namespace работают четыре echo endpoint: TCP и UDP на 127.0.0.1 и ::1.
Тестовый клиент обращается к 198.18.0.2 и 2001:db8:746f::2:40000, для которых
маршрут подтверждён через tofu-test. Ядро читает TUN и перенаправляет назначение
на localhost echo соответствующего семейства. Это проверяет TUN TCP/IP stack и
direct outbound; соединение с удалённым прокси-сервером здесь не проверяется.

Для каждой оси клиент отправляет 32 случайных байта и получает ровно
`tofu-tun:` + исходную нагрузку. Дополнительно проверяется рост обоих счётчиков
пакетов RX/TX TUN. После остановки ядра echo-серверы остаются доступны на loopback,
но запросы к тестовому адресу должны завершиться ошибкой. Проверка повторяется
после нового запуска ядра, отдельно для SIGTERM и SIGKILL.

## Результаты

Использованы те же официальные артефакты и SHA-256, что в
[предыдущем evidence](2026-11-09-foundation.md); digest проверен повторно до запуска.

| Ядро | IPv4 TCP | IPv4 UDP | IPv6 TCP | IPv6 UDP | Cleanup / отрицательный контроль |
| --- | --- | --- | --- | --- | --- |
| Xray 26.10.31 | pass | pass | pass | pass | SIGTERM и SIGKILL: pass |
| sing-box 1.14.4 | pass | pass | pass | pass | SIGTERM и SIGKILL: pass |

Каждая ось проверена дважды: в сессии перед SIGTERM и после перезапуска перед
SIGKILL. Всего 16 успешных echo-проб и 16 отрицательных контролей без ядра.

Python unittest включает локальные echo endpoints всех четырёх осей,
остановку серверных потоков и отказ при усечённом TCP-ответе.
Существующая регрессия process-group cleanup расширена прерыванием запуска:
сигнал инъецируется после создания реального child, но до возврата Popen.
Это воспроизводит найденную гонку при присвоении handle. На предыдущей реализации
проверка падает; с отложенной доставкой исключения и finally cleanup проходит.
Обработчики сигналов восстанавливаются после выхода.

## Ограничения и внешнее состояние

- Это частичное evidence G-LINUX. Не проверены DNS/Fake-IP, приватный VPN,
  атрибуция executable, внешние proxy outbounds, производительность и минимальный
  набор capabilities при systemd hardening.
- Состояния маршрутов/адресов живут только в одноразовом namespace. Удаление
  интерфейса не доказывает очистку будущего host-helper: такого helper пока нет.
- Внешний CI PR #1 не запустил ни одного шага: GitHub сообщил
  `The job was not started because your account is locked due to a billing issue.`
  Это внешний blocker для CI и merge; результаты не объявляются зелёным GitHub CI.
- Локально выполняются Rust tests/fmt/clippy/release, Ruff и Python unittest.
  Android, G-DNS/G-APP/G-POWER/G-UI остаются открытыми.

В дополнение к первоначальному отчёту проверена заявленная минимальная версия:
Rust 1.85.1 — `test --workspace --locked` (5 integration + 1 doctest),
`clippy --workspace --all-targets --locked -- -D warnings`, `fmt --all -- --check`
и `build --workspace --release --locked` проходят. Toolchain установлен только
в собственный transient-каталог, глобальные настройки Rust не менялись.

Независимый read-only reviewer проверил diff `59cd29d..024346e`: блокирующих
findings не обнаружил. Реальные engine-прогоны и результаты тестов reviewer
принял по evidence root; повторно их не запускал.

Источники схем:
[Xray FreedomConfig](https://github.com/XTLS/Xray-core/blob/v26.10.31/infra/conf/freedom.go),
[sing-box route actions](https://sing-box.sagernet.org/configuration/route/rule_action/).
