# Системная конфигурация TUN — 10 ноября 2026

База: `3342b29` (PR #36 влит после всех пяти зелёных jobs обоих workflows).
Кандидат: `feat/system-tun-configuration`; runtime system service не установлен.

## Проверки кандидата

- Rust workspace: **461 passed, 0 failed, 16 ignored**, 44 suites.
- Workspace clippy all-targets -D warnings, fmt: PASS.
- Python: **115 tests**, PASS; Ruff 0.17.4 format/check: PASS.
- Journal: **26 integration tests**, включая Address parent, неверные семейства/
  границы, обратный порядок cleanup и отказ родителю при ошибке ребёнка.
- Actual TunConfig в fresh namespace: MTU/up, две семьи с NOPREFIXROUTE,
  отдельные таблицы и RPDB lookup fixture, reopen/неизвестные intents, exact
  cleanup, сохранение foreign адресов/routes/rules и kernel LL provenance.
- Повторно запущены реальные owned-tun, public-block, durable-policy (шесть
  SIGKILL-границ) и service/backend через canonical namespace launcher.
- Скомпилированный Xray VLESS профиль прошёл TUN → VLESS server → echo:
  TCP/UDP IPv4/IPv6, отсутствие трафика без peer, SIGKILL client и полный cleanup.
  Test peer разрешает только свои синтетические loopback echo endpoints.
- Новый адаптированный sing-box прошёл ту же матрицу: глобальные адреса и
  NOPREFIXROUTE сохранены, нет неучтённых routes/rules после SIGKILL,
  журнал пуст после очистки.
- Обычный sing-box и предыдущий native adapter отклоняют новый режим по
  неизвестному external_configuration на config-check, до сетевых эффектов.

Все сетевые сценарии — отдельные user/net/mount/pid namespaces, private
/run и /tmp, proc/sysfs. Host netns исключён до первого эффекта. Используются
только статические fixture-профили; подписки, HWID и ключи не читались.
Полные логи: `local/validation/system-tun/verified-namespace-*.log`.

## Причина минимального Go-патча

У stock sing-box 1.14.4 AutoRoute=false не исключает IPv6 default route и OIF-rule
в случайной таблице. Новый строгий teardown обнаружил их и отказал в удалении;
ошибка воспроизведена и подтверждена upstream sing-tun 040db71fcd33.

Patch `external-tun-configuration.patch` экспортирует существующий EXP-режим
библиотеки с typed JSON и валидацией несовместимых routes/DNS options. False
сохраняет upstream-поведение. MTU/up/offload остаются штатными операциями;
ownership/ifindex и минимальные права worker ещё проверяются полноценным host.

Версионированный артефакт:
`local/artifacts/sing-box-external-tun-versioned-v1.14.4/sing-box`.
SHA-256: `4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d`.
Source commit `c005f35300ce76427dc285481ec5695f5b179cd2`, source tag v1.14.4;
manifest подтверждает patch hashes и external_tun_configuration. Go module-файлы
не изменены. Builder задаёт source version через linker, проверяет CLI version
перед публикацией и копию по SHA-256; unknown version не публикуется.
Go `protocol/tun` и `option` tests и 15 builder Python tests прошли.
Bit reproducibility и весь strict DNS gate по-прежнему не заявляются.

## Независимое ревью

Исправлены P2: удаление чужих зависимостей вместе с owned TUN; захват пустой
таблицы, используемой чужим rule. Namespace-регрессии и узкий recheck прошли.
Отдельное review native-контракта потребовало включить protocol/tun tests в CI;
пакет добавлен в обычный/race прогоны. Version pin и manifest binding проверены.

CI дополнен TunConfig namespace и compiled TUN адаптера; результат удалённого
кандидата фиксируется отдельно. Системный worker/service/controller/GUI,
production RPDB, DNS/Fake-IP и полная Linux-матрица остаются открыты.
P12/P13 не закрыты; общий план остаётся **3/26 (11,5%)**.

## Исправление гонки CLI-теста в PR #37

Rust job `129596536204` упал на записи stdin: CLI отверг неверные аргументы
до чтения входа, и тест получил BrokenPipe. Тестовый launcher допускает только
эту ошибку, затем сохраняет проверки статуса, пустого stdout и безопасного stderr.
Добавлен большой вход при неизвестном аргументе. Продуктовый CLI не изменён.
Все **27 тестов yonik-engine**, fmt и clippy all-targets с запретом warnings прошли;
независимое узкое ревью не выявило замечаний. Логи: `local/validation/ci37/`.
