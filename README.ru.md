# tofu

[English](README.md) · [Русский](README.ru.md)

Нативный прокси-клиент с открытым исходным кодом. Сначала Linux
(CachyOS/Arch, KDE Plasma, Wayland), затем Android. Запланированы Xray-core и
sing-box, TUN, Fake DNS, подписки, маршрутизация по приложениям и работа в трее.

**Ранняя разработка:** в репозитории есть спецификации, начальная переносимая
Rust-библиотека маршрутизации и изолированные smoke-тесты ядер. Готового
desktop-приложения, установщика и рабочего сетевого адаптера пока нет.

### Что реализовано

- Проверка и сопоставление канонических IPv4/IPv6-подсетей.
- Порядок решений: защищённые приватные VPN-сети → обнаруженная LAN → личные
  правила → пресеты → действие по умолчанию.
- Условия executable и назначения: И между группами, ИЛИ внутри группы.
- Блокировка приватного назначения при недоступности его VPN-маршрута.
- Проверки создания и удаления TUN Xray/sing-box в отдельных Linux namespace.

Расчёт политики не доказывает фактический путь пакета. Домены, DNS/Fake-IP,
импорт, хранение, компиляторы конфигураций ядер, helper, GUI и трей ещё предстоит
реализовать. Выбор GUI toolkit остаётся открытым.

### Разработка

Rust 1.85+ (edition 2024). У начальной библиотеки нет сторонних Rust-зависимостей.
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
```

Тесты не меняют сеть хоста и системный DNS. Если бинарники размещены в transient,
удерживайте `flock -s` корня task-каталога весь запуск.
[Версии, хэши и результаты](docs/validation/2026-11-09-foundation.md).
Smoke проверяет конфигурацию и lifecycle интерфейса; полные G-LINUX/G-DNS/G-APP
ещё не пройдены.

Не коммитьте ссылки подписок, credentials и настоящие конфигурации серверов.
Каталог `local/` исключён из Git для приватных файлов разработки.

### Документация и лицензия

- [Требования, архитектура и roadmap](docs/specs/README.md)
- [План начального этапа](docs/superpowers/plans/2026-11-09-linux-foundation.md)
- [GNU GPL v3.0](LICENSE); уточнение SPDX и notices зависимостей требуется перед поставкой.
