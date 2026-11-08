# Начальный этап: результаты проверок 9 ноября 2026

## Область доказательства

Это первый срез общей библиотеки и engine smoke. Полные gates G-LINUX, G-DNS,
G-APP, G-ANDROID и G-POWER не пройдены. Готового приложения нет.

## Среда

- Linux x86_64, Rust/Cargo 1.97.0; edition 2024, заявленный MSRV 1.85 отдельно не запускался.
- NetworkManager и systemd-resolved активны; `resolv.conf mode: foreign`.
  Активность resolved не доказывает управление системным DNS. Сеть хоста не менялась.
- `/dev/net/tun` доступен; `unshare --user --map-root-user --net` разрешён.
- Android target не установлен; Android compile-check пока не выполнен.
- До начала работы Xray/sing-box отсутствовали в PATH. Использованы отдельные
  официальные бинарники в transient с lock корня, без установки в систему.

## Закреплённые артефакты smoke

| Ядро | Артефакт | SHA-256 |
| --- | --- | --- |
| Xray 26.10.31 | Xray-linux-64.zip | e4bd9119f50ae7c41658b1abd0679ae06f98bd864a2907ff33f2f2cf7081b214 |
| sing-box 1.14.4 | sing-box-1.14.4-linux-amd64.tar.gz | 699a1b510aa4396b2176d32d1181920399ebf1e267835da6f3b598d0f7c24968 |

Источники: [Xray release](https://github.com/XTLS/Xray-core/releases/tag/v26.10.31),
[sing-box release](https://github.com/SagerNet/sing-box/releases/tag/v1.14.4).
Хэши скачанных архивов сравнены с digest из GitHub Release API до запуска.
Это проверка целостности относительно релиза, не независимая подпись издателя.

## Выполнено

- `cargo test --workspace`: 5 поведенческих тестов и compile-fail doctest.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo fmt --all -- --check` и release-сборка.
- Xray `run -test` и sing-box `check` для синтетических fixtures.
- Оба ядра: создание TUN в новом network namespace, затем отсутствие интерфейса
  после SIGTERM и SIGKILL. После каждого завершения список интерфейсов равен
  исходному `{lo}`. Проверка не меняет DNS и не устанавливает host routes.

Поведенческие тесты покрывают CIDR/семейства IP, AND/OR условий,
неизвестный executable, порядок правил, недоступный VPN и пересечение приватных
сетей. Reviewer обнаружил возможность задать PrivateVpn как пользовательское
действие; исправление разделяет Action и Route. Compile-fail тест запрещает
восстановление такого состояния API.

## Приватная подписка

Источник предоставлен пользователем. URL, адреса, ключи, имена профилей и
исходные конфигурации не сохранялись в репозитории или evidence.

- HTTP 200, 1116 байт, Base64, три непустые строки: две VLESS, одна пропущена
  как не-VLESS в локальном диагностическом эксперименте.
- TCP connect до обеих VLESS-точек успешен: приблизительно 2 и 113 мс,
  по одной выборке, без выделения DNS-времени. Это не benchmark.
- По одному HTTPS-запросу к `https://www.gstatic.com/generate_204` через
  выделенный localhost SOCKS inbound Xray: обе проверки дали curl exit 28,
  HTTP 000 после 12 секунд. TLS-проверка не отключалась, прямой fallback не использовался.
- Конфигурации эксперимента передавались Xray через stdin, логи ядра отключены.
  Проба не является продуктовым импортёром или compiler. Причина таймаутов
  не локализована: недоступность сервера, транспорт, параметры либо endpoint.
  Профили не объявляются рабочими или невалидными по этому результату.

Начальная попытка передать конфигурацию через memfd не запустила ядро:
Xray отвергает путь без поддержанной extension, даже с `-format json`.
На синтетическом конфиге подтверждён вход `-c stdin:`, после чего выполнены
указанные сетевые пробы. Секретные данные на диск для этого не записывались.

## Референс Clash Verge Rev

По предложению пользователя изучены исходники commit
`77a2df70c2aa5d3c7acc823b8484eba70c3d2860`:

- [network_watch.rs](https://github.com/clash-verge-rev/clash-verge-rev/blob/77a2df70c2aa5d3c7acc823b8484eba70c3d2860/src-tauri/src/core/network_watch.rs): реакция на изменения сети и повторное применение system proxy.
- [tray/speed_task.rs](https://github.com/clash-verge-rev/clash-verge-rev/blob/77a2df70c2aa5d3c7acc823b8484eba70c3d2860/src-tauri/src/core/tray/speed_task.rs): отдельно управляемая задача скорости трея и остановка при отключении функции.
- [enhance/tun.rs](https://github.com/clash-verge-rev/clash-verge-rev/blob/77a2df70c2aa5d3c7acc823b8484eba70c3d2860/src-tauri/src/enhance/tun.rs): согласование TUN и DNS в конфигурации mihomo.

Это ориентиры для следующих этапов. Код не копировался; семантика mihomo
не доказывает эквивалентность Xray/sing-box. Трей tofu по спеке должен работать
без обязательного опроса скорости в фоне.

## Дальше

1. Реальные TCP/UDP IPv4/IPv6 потоки через TUN на изолированном стенде.
2. Атрибуция executable и одинаковая маршрутизация на обоих ядрах (G-APP).
3. DNS/Fake-IP и приватный VPN, сбои/восстановление и журнал владения ресурсами.
4. Проверка переносимого crate под Android и выбор pinned wrappers.
5. Продуктовый импорт/компиляторы/контроллер; затем согласование GUI.
