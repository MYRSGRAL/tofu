# tofu

[English](README.md) · [Русский](README.ru.md)

Нативный прокси-клиент с открытым исходным кодом. Сначала Linux
(CachyOS/Arch, KDE Plasma, Wayland), затем Android. Запланированы Xray-core и
sing-box, TUN, Fake DNS, подписки, маршрутизация по приложениям и работа в трее.

**Ранняя разработка:** в репозитории есть спецификации, переносимые
Rust-библиотеки, offline CLI импорта/компиляции и изолированные тесты ядер. Готового
desktop-приложения, установщика и рабочего сетевого адаптера пока нет.

### Что реализовано

- Проверка и сопоставление канонических IPv4/IPv6-подсетей.
- Порядок решений: защищённые приватные VPN-сети → обнаруженная LAN → личные
  правила → пресеты → действие по умолчанию.
- Условия executable и назначения: И между группами, ИЛИ внутри группы.
- Блокировка приватного назначения при недоступности его VPN-маршрута.
- Проверки lifecycle TUN и реальных TCP/UDP IPv4/IPv6 echo-потоков Xray/sing-box
  в отдельных Linux namespace.
- Нативные executable-правила, включая два разных пути с одинаковым basename.
- Переносимая модель сессии с отклонением устаревших событий и явным cleanup.
- Одноразовый PreparedPlan с привязкой к поколениям окружения и digest конфига.
- Ограниченный URI/Base64 импорт VLESS, VMess, Trojan, Shadowsocks, HTTP CONNECT и SOCKS5.
- Строгая компиляция одного сервера для Xray 26.10.31 и sing-box 1.14.4:
  loopback SOCKS, проверка TLS и явные ошибки неподдерживаемых параметров.

Rust-библиотека рассчитывает политику; изолированные стенды дают описанные ниже
ограниченные сетевые доказательства. Домены, DNS/Fake-IP,
сетевые обновления подписок, хранение, компиляцию политики для ядер, helper, GUI и трей ещё предстоит
реализовать. Выбор GUI toolkit остаётся открытым.

`tofu-application` реализует начальный цикл подключения, проверки и остановки.
Host обязан выбрать уникальный controller ID, пока возможны callbacks старого
экземпляра. I/O, recovery, перенастройка и сохранение состояния ещё не
реализованы; состояние reducer само по себе не доказывает исправность сети.
Подготовка проходит через `prepare/apply`; прямое `Event::Validated` удалено.
Доверенный host передаёт свежие поколения и SHA-256 проверенного immutable artifact.
Это не заменяет авторизацию ОС и согласованное применение изменений самим host.

### Разработка

Rust 1.85+ (edition 2024). Domain/application используют только std; у импортера
стандартные библиотеки Base64, URL/IDNA и JSON. Cargo.lock фиксирует MSRV-совместимые версии.
Сборки размещаются вне checkout. На рабочей машине проекта:

```sh
tofu_build=/var/tmp/codex-myrs/transient/tofu-dev
mkdir -p "$tofu_build"
cargo fmt --all -- --check
flock -s "$tofu_build" env CARGO_TARGET_DIR="$tofu_build/target" cargo test --workspace --locked
flock -s "$tofu_build" env CARGO_TARGET_DIR="$tofu_build/target" cargo clippy --workspace --all-targets --locked -- -D warnings
```

Для smoke-тестов нужны Linux, Python 3.12+, `ip`, `unshare`, `/dev/net/tun`,
разрешённые непривилегированные user/network namespace и проверенные бинарники:

```sh
python tools/engine_smoke.py xray /абсолютный/путь/к/xray
python tools/engine_smoke.py sing-box /абсолютный/путь/к/sing-box
python tools/engine_traffic.py xray /абсолютный/путь/к/xray
python tools/engine_traffic.py sing-box /абсолютный/путь/к/sing-box
```

Тесты не меняют сеть хоста и системный DNS. Если бинарники размещены в transient,
удерживайте `flock -s` корня task-каталога весь запуск.
[Версии, хэши и результаты](docs/validation/2026-11-09-foundation.md).
Smoke проверяет конфигурацию и lifecycle интерфейса. Traffic suite передаёт
случайную нагрузку через TUN к перенаправленным локальным echo-серверам,
проверяет счётчики интерфейса и отрицательный контроль после остановки.
[Результаты потоковых проверок](docs/validation/2026-11-09-tun-traffic.md).
Полные G-LINUX/G-DNS/G-APP ещё не пройдены: нужны proxy outbounds, DNS,
интеграция приватного VPN и расширенная проверка атрибуции процессов.

Для начальной проверки атрибуции есть отдельный стенд:

```sh
tofu_apps=/var/tmp/codex-myrs/transient/tofu-apps
mkdir -p "$tofu_apps"
flock -s "$tofu_apps" python tools/engine_apps.py xray /абсолютный/путь/к/xray --work-dir "$tofu_apps"
flock -s "$tofu_apps" python tools/engine_apps.py sing-box /абсолютный/путь/к/sing-box --work-dir "$tofu_apps"
```

Он проверяет точные executable-пути последовательных TCP/UDP IPv4/IPv6 потоков
одного UID. Охват desktop-приложений и полный G-APP этим не подтверждаются.
[Результаты executable-проверок](docs/validation/2026-11-09-process-routing.md).

Не коммитьте ссылки подписок, credentials и настоящие конфигурации серверов.
Каталог `local/` исключён из Git для приватных файлов разработки.

### Диагностика offline-импорта

```sh
flock -s "$tofu_build" env CARGO_TARGET_DIR="$tofu_build/target" cargo run --quiet --locked -p tofu-import < local/subscription.txt
```

На входе тело подписки, не URL загрузки. CLI выводит только счётчики, протоколы,
статические коды ошибок и номера строк. Exit 0 означает получение отчёта, включая
частично ошибочный импорт; профили не активируются.
Лимиты: 8 MiB входа, 32 MiB decoded, 20 000 URI, 64 KiB на URI, глубина JSON 64.
Native JSON сохраняется opaque после проверки синтаксиса; извлечение managed
outbounds ещё не реализовано. Отдельный компилятор проверяет возможности URI;
host всё ещё обязан выполнить config-check закреплённым ядром до активации. Фатальные ошибки
сохраняют исходные байты в API; обрезанный/сбойный поток помечается неполным.
[Результаты и ограничения импортера](docs/validation/2026-11-09-subscription-import.md).

### Компиляция одного сервера

```sh
flock -s "$tofu_build" env CARGO_TARGET_DIR="$tofu_build/target" cargo run --quiet --locked -p tofu-engine --bin tofu-compile -- xray --server 0 < local/profile.txt
```

Обычный вывод — метаданные (`compiled_not_checked`), а не результат подключения.
Нужен полный импорт URI без отклонённых записей; индекс `--server` начинается с нуля.
Для другого ядра укажите `sing-box`, для смены порта 10808 — `--listen-port`.
Флаг `--emit-secret-config` явно выводит чувствительный JSON в stdout: передавайте
его только доверенному потребителю, не в логи или публичный issue. Debug типа
`CompiledConfig` скрывает содержимое; JSON доступен через `write_sensitive`.

Поддерживаются шесть импортируемых протоколов, совместимые TCP/WS/gRPC,
XHTTP у Xray, TLS с проверкой сертификата по системным CA, VLESS TCP REALITY и Vision.
Неизвестные/неприменимые параметры, небезопасный TLS, legacy VMess alterId,
плагины и неподдерживаемые транспорты/шифры явно отклоняются. Публичный plaintext
VLESS у Xray запрещён; тесты на приватных IP разрешены. SOCKS поверх TLS и
активация native JSON ещё не реализованы. Listener всегда `127.0.0.1`, outbound
один, прямого fallback нет. Компилятор не включает TUN, правила маршрутизации,
системный прокси или DNS и не запускает ядро.

Синтетические config-check, TCP/UDP IPv4/IPv6 и отрицательные проверки TLS:

```sh
flock -s "$tofu_build" env TMPDIR="$tofu_build" python tools/engine_compile.py "$tofu_build/target/debug/tofu-compile" /абсолютный/путь/к/xray /абсолютный/путь/к/sing-box
```

Нужен `openssl` в дополнение к namespace-инструментам и закреплённые версии ядер.
[Результаты и ограничения компиляторов](docs/validation/2026-11-09-engine-compiler.md).

### Документация и лицензия

- [Требования, архитектура и roadmap](docs/specs/README.md)
- [План начального этапа](docs/superpowers/plans/2026-11-09-linux-foundation.md)
- [GNU GPL v3.0](LICENSE); уточнение SPDX и notices зависимостей требуется перед поставкой.
