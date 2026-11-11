# План завершения yonik

Начальная инвентаризация: 10 ноября 2026, UI `85b2a8e`. Статусы актуализированы 11 ноября 2026 по принятому main `d0a281b`. Цель — пройти весь порядок из
[спецификации](specs/validation-and-roadmap.md): рабочий Linux-продукт,
проверенная поставка, затем Android. Пункты и критерии ниже уточняют порядок
исполнения, не заменяя требования FR и обязательные матрицы приёмки.

## Как считаем готовность

Всего **26 пунктов**: **3 готовы**, **21 частично выполнен**, **2 не начаты**.
Остаётся полностью закрыть **23 пункта**, включая отложенные Android P03/P26.
Это учёт полных критериев, а не процент реализации Linux: частичные пункты
различаются по объёму и не пересчитываются в процент готовности продукта.
Частичные результаты учитываются в описаниях, но не получают условные 50–90%.

Ранние G-LINUX/G-DNS/G-APP включены в соответствующие сетевые пункты, поэтому
одна интеграционная проверка не посчитана дважды. Android compile-check и
технический Android-host разделены: успешная компиляция не доказывает работу VPN.

Статус «готов» требует проверяемого результата по всему критерию пункта.
Исторические evidence подтверждают только описанные в них срезы. После правок
обязательны релевантные проверки; green CI и независимое ревью нужны перед
слиянием согласно действующим договорённостям. Нельзя закрыть весь план только
по наличию GUI, количества unit-тестов или namespace-прототипа.

## Текущая очередь Linux — 11 ноября 2026

Android P03/P26 отложены пользователем. Следующий порядок относится только к Linux;
новые Android-инструменты и зависимости не устанавливаются. Принятый main
`d0a281b` содержит принятые ниже срезы. Открытые #83/#85/#87/#88
в этот результат не входят.

- [Системный runtime #49](validation/2026-11-10-system-session-matrix.md)
  прошёл изолированную libtest-матрицу 12/12: оба ядра,
  TCP/UDP IPv4/IPv6, kill switch on/off и stop/restart/worker_exit. Это отдельное
  evidence от [production helper #73](validation/2026-11-10-production-helper-vm.md):
  его actual 4/4 относится к Xray/VLESS с kill switch, авторизации и восстановлению.
  GUI → production helper, полный transport обоих ядер и минимальные права ещё
  не приняты вместе. `system_tun:false` сохраняется.
- [Scheduler в DesktopHost](validation/2026-11-10-desktop-subscription-scheduler.md),
  [persisted due/CAS](validation/2026-11-10-persisted-subscription-due.md) и
  [private-bus reconnect](validation/2026-11-10-desktop-private-bus.md) реализованы.
  Остаются фоновые окна и физическая энергоприёмка; private bus не заменяет её.
- Каталог приложений и draft-редактор приняты, включая
  [revision/ACK после Stop](validation/2026-11-10-routing-cancel-ack.md).
  Нужны UI диагностики пересечений/объяснения маршрута и actual Apply.
- [Batch GUI](validation/2026-11-10-batch-gui/unavailable-favorite.md) принят:
  10 000 профилей, 30 000 TCP-соединений, network epoch/cancel и исчезнувшее
  избранное. Это не измерение плавности в живой Plasma.
- Рабочие native Real/Fake-IP и IPv6 реализованы;
  [disabled-family oracle](validation/2026-11-11-dns-family-gate-review.md) проверен
  отрицательной мутацией. TTL/epochs, exclusions, network reset и системный G-DNS
  остаются открытыми. [G-APP baseline](validation/2026-11-11-g-app.md) подтверждает
  executable-маршруты; dedicated-UID admission, упаковки/PID reuse и owner-cost
  требуют продолжения.
- Приняты [ru-direct snapshots/rollback](validation/2026-11-10-preset-snapshots.md),
  [backup schema/replay checks](validation/2026-11-10-storage-backup.md),
  [Secret Service failure matrix](validation/2026-11-10-secret-service-failures.md),
  [active SOCKS tray](validation/2026-11-10-active-tray.md) и
  [74-кодовый RU/EN каталог](validation/2026-11-11-p11-message-catalog.md).
  Product wiring, настоящие failure paths, runtime log UI и live KDE gates
  учитываются отдельно от этих срезов.
- [Clean Arch build](validation/2026-11-10-arch-clean-build.md) доказан для source
  `eec7a56`, который предшествует новым auth/TUN fixes. Финальная интегрированная
  поставка требует свежей фиксированной базы и всех системных проверок.
  Rust notices содержат 225 packages без missing license; exact ELF/Qt/Go
  composition/provenance остаётся отдельной незавершённой приёмкой.

Сейчас независимы четыре направления: System GUI recovery guard (#88), production
G-APP/helper admission, composition/notices (#85) и P-04 ресурсная матрица
импорта/компиляции 1x/10x. Общие backend/helper/DNS контракты имеют одного writer.
Следом после освобождения UI идут редактор diagnostics и журнал фактических событий;
после стабилизации host/DNS — полная Linux-матрица и integrated packaging.
Слияние каждого PR требует независимого review и зелёного CI на его точном head.
Число слитых PR не заменяет закрытие полного Pxx или FR.

## История первой очереди (10 ноября 2026)

Следующий раздел сохранён как история начальной выдачи. Формулировки «следующий»,
«ещё нужен» и Android-порядок в нём не являются текущими заданиями; актуальные
остатки указаны выше и в таблицах P01–P26.

Продолжается полное выполнение спецификации по
[плану волн](superpowers/plans/2026-11-10-spec-completion-waves.md).
Первая волна влита в main: PR #43 (`374ec75`) — переводы и CI,
#44 (`3768988`) — переносимый scheduler, #45 (`830af0d`) — матрица импорта,
#46 (`e33fa19`) — проверки восстановления хранения. На принятых heads все пять
jobs зелёные; замечания независимых reviews исправлены. P10 перешёл из «не начат»
в «частично»; число полностью закрытых пунктов и **3/26 (11,5%)** не изменились.
Runtime-срез подготовлен к независимому review: типизированный IPC, общий journal
и worker-first recovery проверены unit tests и namespace gates. Installed session
VM-матрица и TERM→KILL не приняты; `system_tun:false` сохраняется.
[Фактические проверки и остаток](validation/2026-11-10-system-session-runtime.md).
Следующая волна: системный runtime, Linux scheduler host, постоянные настройки,
Arch-пакет и ранний Android gate. Завершение волны не завершает весь проект.
После исследования Android пользователь отложил его реализацию и установку
инструментов: текущая очередь продолжает Linux. P03/P26 остаются открытыми.

Повторная проверка 10 ноября: desktop entry указывает на установленную сборку
`85b2a8e`, SHA-256 бинарника совпал с опубликованным evidence. На этой сборке
снова прошли 142 GUI-действия при 380×600 и масштабе 150% в offscreen-окне,
включая проверки отсутствия удалённых controls. Evidence:
`local/validation/whole-ux/recheck-2026-11-10/`. Это функциональная проверка;
она не заменяет замеры плавности в живой Wayland-сессии P23.

1. Последние UI-правки и desktop CI завершены: PR #28 влит как `63fb1a5`
   после успешных push/PR workflows (все 5 PR jobs). UI установлен и проверен.
   Причина сбоя CI — путь сокета 107 байт; короткий приватный runtime исправляет запуск.
2. Реестр системных ядер влит через PR #29 (`628d5c3`), CI зелёный.
   Одноразовые peer-bound artifacts влиты PR #30 (`d585b01`), CI зелёный.
   Общий бюджет buffers/snapshots реализован в `73a6ab2`: локально 393 Rust-теста
   прошли, независимое ревью выполнено. PR #31 влит как `acff6c5` после всех
   пяти зелёных jobs в обоих workflows. Выявленный CI сбой очистки launcher на
   relatime исправлен в `ed770be`, все 100 Python-тестов прошли.
   [Nftables executor](specs/helper-nftables-executor.md) влит PR #32 (`dd82114`)
   после всех пяти зелёных jobs, включая fixes `0a90a8e`/`d479217`.
   [Evidence базового executor](validation/2026-11-10-helper-nftables.md).
   Добавлена [типизированная блокировка](specs/helper-public-block.md) с атомарной
   заменой правил: реальные TCP/UDP IPv4/IPv6, VPN/LAN, cold NDP, DNAT и Unlock
   [проверены в namespace](validation/2026-11-10-helper-public-block.md),
   PR #33 влит как `37401a4` с зелёным CI.
   Добавлено [durable хранение и восстановление](specs/helper-policy-recovery.md):
   schema v2, desired/previous/Unlock и шесть SIGKILL-границ в namespace;
   [evidence](validation/2026-11-10-helper-policy-recovery.md): 427 workspace tests,
   три namespace-сценария и независимое review.
   PR #34 влит как `cef982a`, все пять jobs зелёные.
   Реализован [IPC v1 и public-protection host](specs/helper-service.md),
   добавлены service packaging и cgroup fence; [проверки](validation/2026-11-10-helper-service.md)
   охватывают private-bus/реальный nft и inert restart. PR #35 влит как `e61c7e8`,
   все пять jobs зелёные. [Владение persistent TUN](specs/helper-owned-tun.md)
   реализовано отдельным executor: create/alias/reopen/cleanup и совместимость
   обоих ядер [проверены в namespace](validation/2026-11-10-owned-tun.md).
   PR #36 влит как `3342b29`, все пять jobs зелёные после исправления
   namespace staging и чистого Cargo target в Ubuntu CI.
   [Адреса/маршруты и системный компилятор](specs/helper-tun-configuration.md)
   реализованы: [actual TUN→VLESS→echo](validation/2026-11-10-system-tun-configuration.md)
   проверен для Xray и адаптера sing-box с external configuration; PR #37 влит
   как `e9c4c1` после всех пяти зелёных jobs обоих workflows и исправления
   гонки CLI-теста `8a148c2`.
   [Управляемая RPDB](specs/helper-tun-rpdb.md) добавляет durable правила и
   mark bypass; оба compiler-профиля проверены с peer за veth в отдельном netns.
   PR #38 влит как `3a48c40`, все пять jobs обоих workflows зелёные.
   Срез [worker runtime](specs/helper-worker-runtime.md) добавляет sealed-FD
   Check/Run, fixed systemd units и supervisor. Actual TUN/RPDB traffic обоих
   ядер проверен с отдельным UID и только NET_ADMIN; Check без capability.
   Установленные units/cgroup и dedicated UID проверены в одноразовой Arch VM:
   [три actual gates](validation/2026-11-10-installed-worker.md) прошли, включая
   setsid-потомка, helper SIGKILL/recover и TUN traffic обоих ядер. Остаются
   TERM→KILL для упрямого потомка и подключение runtime к session host.
   PR #39 влит как `6355150`, PR #40 — как `49debdd`; все пять jobs обоих
   workflows зелёные на соответствующих heads. Следующий срез:
   [типизированный профиль и неизменяемый сетевой план](specs/helper-session-network.md).
   Шесть протоколов DTO и configure/capture/external-check/cleanup проверены;
   [evidence](validation/2026-11-10-session-plan.md). Сам SystemSessionHost ещё нужен.
   Следом нужны системная установка/клиент, live reconcile и TUN-сессия с явным
   владением ресурсами и cleanup по [плану интеграции](superpowers/plans/2026-11-10-system-session-integration.md);
   затем DNS/Fake-IP и атрибуцию приложений.
3. Интегрировать приватные VPN/LAN/ru-direct, сценарии смены сети и полную
   Linux-матрицу. При включённом kill switch после исчерпания retries сеть
   блокируется; при выключенном восстанавливается обычная сеть.
4. Довести продуктовый GUI, редакторы, сохранение настроек, трей, диагностику,
   производительность и Arch-поставку. Основная часть остаётся на Rust;
   Go допустим только для минимальных совместимых патчей самих ядер.
5. Ранний Android runtime-gate P03 проверять параллельно Linux-основанию, до
   окончательного закрепления общей архитектуры. Полное Android-приложение и его
   поставка P26 идут после Linux v1, как требует исходная последовательность.

## Основа и общая логика

| ID | Пункт | Статус | Критерий полного завершения и текущий остаток |
| --- | --- | --- | --- |
| P01 | Требования и архитектурные контракты | Готов | Зафиксированы FR, сетевые инварианты, границы компонентов, Android-риски и бюджеты. Evidence: [требования](specs/requirements.md), [приёмка](specs/validation-and-roadmap.md). Реализация gates считается отдельно. |
| P02 | Переносимая библиотека: Android compile-check | Готов | Общая Rust-часть компилируется для `aarch64-linux-android` без Linux-адаптеров; проверка включена в CI. Evidence: [snapshots](validation/2026-11-09-subscription-snapshots.md), [workflow](../.github/workflows/check.yml). Это не Android-приложение. |
| P03 | Технический Android-host / G-ANDROID | Не начат | Оба engine через FFI, TUN FD/protect, TCP/UDP, stop/revoke, lifecycle и package callbacks; закреплённые wrappers. Остаток — runtime-host и измеренные результаты. [Контракт](specs/platforms.md). |
| P04 | Импорт URI/JSON/Base64 и ручной ввод | Частично | Есть ограниченный parser, сохранение исходника и review. Остаются все требуемые варианты ввода и табличные/property/fuzz-проверки размеров, вложенности, IDNA/IP и неизвестных опций. [Evidence](validation/2026-11-09-subscription-import.md), [тесты](../crates/yonik-import/tests). |
| P05 | Подписки, обновления и идентичность профилей | Частично | CAS, rename/delete, отмена, HWID и последний snapshot реализованы. Завершить proxy-загрузку без скрытого DIRECT и полную матрицу изменения credentials/пустого/частичного ответа. Плановые обновления — P10. [Snapshots](validation/2026-11-09-subscription-snapshots.md), [identity](validation/2026-11-09-profile-identities.md). |
| P06 | Зашифрованное хранение, миграции и восстановление | Частично | Приняты encrypted repository/CAS, private-bus Secret Service failure matrix и maintenance backup/restore с schema/replay validation. Остаются полный crash/fsync fault injection и реальная login-session приёмка; legacy data/key domains сохраняются. [Backup](validation/2026-11-10-storage-backup.md), [keyring](validation/2026-11-10-secret-service-failures.md). |
| P07 | Общая модель политик и семантика представлений | Частично | Доменная модель и приоритеты протестированы. Завершить полную матрицу AND/OR, стабильных ID, неподдерживаемых селекторов и сохранения семантики при преобразованиях; GUI редакторов — P20. [Код](../crates/yonik-domain/src/policy.rs), [evidence](validation/2026-11-09-policy-routing.md). |
| P08 | Capabilities и компиляторы обоих ядер | Частично | Есть config-check закреплённых ядер и явные отказы для части опций. Закрыть весь заявленный набор профилей/правил, включая ограничения смешанного DNS, без игнорирования параметров. [Evidence](validation/2026-11-09-engine-compiler.md), [routing](../crates/yonik-engine/src/routing.rs). |
| P09 | Lifecycle сессии и PreparedPlan | Частично | Reducer и поколения реализованы. Завершить host-интеграцию: конкурирующие Connect, отмена Validate/Apply, поздние события, повтор Stop, потеря host и подтверждённый cleanup. [Тесты](../crates/yonik-application/tests/session.rs), [evidence](validation/2026-11-09-prepared-plan.md). |
| P10 | Scheduler фоновых обновлений | Частично | Переносимый scheduler, DesktopHost dispatch/review/cancel, событийная NM/UPower policy, persisted due/retry budget и private-bus restart/reopen реализованы. Остаются фоновые окна и полная энергоприёмка. [Evidence](validation/2026-11-10-desktop-private-bus.md). |
| P11 | Ошибки и переводы RU/EN | Частично | Принят общий закрытый каталог 74 code/key, RU/EN QML/tray checks и explicit clipboard export. Controlled snapshot не подтверждает все реальные actor failures; System UI, catalog fallback и полная продуктовая приёмка остаются. [Evidence](validation/2026-11-11-p11-message-catalog.md). |

## Linux-сеть

| ID | Пункт | Статус | Критерий полного завершения и текущий остаток |
| --- | --- | --- | --- |
| P12 | Privileged helper и владение ресурсами | Частично | Production helper executable/system bus/polkit/TUN sandbox, typed client и recovery приняты отдельными срезами. Actual Xray protected4/4 не доказывает combined GUI/оба ядра/full transport. Нужны policy-bearing admission при минимальных правах, host topology reconciliation и полная установленная сессия с cleanup. [Production evidence](validation/2026-11-10-production-helper-vm.md), [IPC](specs/helper-service.md). |
| P13 | Системный TUN обоих ядер / G-LINUX | Частично | [SystemSessionHost](validation/2026-11-10-system-session-matrix.md) прошёл изолированную libtest-матрицу12/12 обоих ядер/kill-switch/stop/restart/worker_exit/TCPUDPv4v6. Production helper Xray4/4 — отдельный ограниченный набор. Нужны combined GUI/production worker/политика/minimal privileges и полная transport/DNS матрица; system_tun:false сохраняется. [Production границы](validation/2026-11-10-production-helper-vm.md). |
| P14 | DNS/Fake DNS / G-DNS | Частично | Единый DnsPolicy и рабочие native Real/Fake-IP IPv4/IPv6 обоих ядер реализованы, unknown mappings и disabled-family oracle проверены. TTL/epochs, positive exclusions, network reset, private DNS runtime revocation и системный D-01…D-07 ещё не приняты. [Evidence](validation/2026-11-11-dns-family-gate-review.md). |
| P15 | Атрибуция приложений / G-APP | Частично | Приняты реальные executable/CIDR маршруты обоих ядер в изолированном стенде. Dedicated worker UID/minimal privileges, policy admission, packaging/PID reuse и стоимость owner lookup ещё не приняты. [Evidence](validation/2026-11-11-g-app.md). |
| P16 | Приватные VPN, LAN и изменения сети | Частично | Read-only наблюдение topology/resolver реализовано. Нужны verified binding WG, split DNS, Apply и сохранение защиты при смене VPN/сети/sleep, затем корректное восстановление. [Наблюдение](validation/2026-11-09-network-observation.md), [topology](validation/2026-11-09-network-topology.md), [resolver](validation/2026-11-09-resolver-host.md). |
| P17 | ru-direct и комплексная Linux-приёмка | Частично | Согласованные immutable ru-direct snapshots с update/install/rollback и восстановлением при повреждении одной версии приняты. Нужны product wiring, actual приоритеты private/LAN/user и полная матрица двух ядер/Fake DNS/IPv6/MTU. [Evidence](validation/2026-11-10-preset-snapshots.md). |

## Приложение и поставка

| ID | Пункт | Статус | Критерий полного завершения и текущий остаток |
| --- | --- | --- | --- |
| P18 | Последние согласованные UI-правки и HWID | Готов | yonik, узкое окно, card+gear, центральная кнопка; нет избранного, footer-кнопок/текста, бейджа и чекбокса анимации. Opt-in HWID стабилен и зашифрован. Установлена `85b2a8e`: по 142 GUI-действия в двух размерах, 6 health-сценариев, review без findings. [UI evidence](validation/2026-11-09-home-cleanup.md), [HWID](validation/2026-11-09-hwid-ui.md). Общая продуктовая приёмка — P19/P23. |
| P19 | Продуктовые экраны и системное подключение | Частично | Local SOCKS и persisted preferences существуют. System controller PR88 пока открыт с recovery-guard finding; fixture/Qt успешный путь не заменяет production GUI/TUN/transport acceptance. Управление версиями и полные пользовательские пути ещё нужны. [Контракт](specs/ui-backend-contract.md). |
| P20 | Два редактора правил и каталог приложений | Частично | Приняты два представления draft Policy, каталог, encrypted CAS, revision/ACK и фиксы повторного Confirm/Stop. Нужны диагностика пересечений, объяснение маршрута, atomic actual Apply и полная A-04/A-07/A-08. [Evidence](validation/2026-11-10-routing-cancel-ack.md). |
| P21 | Трей и desktop lifecycle | Частично | Базовые команды и single instance есть. Пройти T-01…T-04 на KDE, restart Plasma, отсутствие трея, ошибки cleanup, закрытие окна и «Выйти и отключиться». [Evidence](validation/2026-11-09-native-desktop.md), [приёмка](specs/validation-and-roadmap.md). |
| P22 | Пакетные проверки серверов и диагностика | Частично | Приняты batch GUI и actual 10k/30k TCP, stale source/network epoch/cancel, доступные цели при исчезнувшем избранном. HTTPS selected-outbound и полные endpoint/error/diagnostic требования проверяются отдельно. [Evidence](validation/2026-11-10-batch-gui/unavailable-favorite.md). |
| P23 | Выбор toolkit, G-UI и G-POWER | Частично | Qt UI и отдельный стенд есть. Нужны реальные замеры Qt/GTK4/Slint в разблокированной сессии, продуктовая приёмка ввода/clipboard/масштабов/трея и P-01…P-08 с idle/hidden/attribution. [Стенд](../bench/ui/README.md), [бюджеты](specs/performance.md). Экран блокировки и питание без согласования не менять. |
| P24 | Arch-поставка, обновления и откат | Частично | Принят clean Arch VM build source eec7a56 и install/Qkk/Qt smoke; package lifecycle update/rollback/uninstall проверен отдельно. Нужен свежий integrated package с production helper/registry/GUI и теми же гарантиями сохранности данных/чужих ресурсов. [Evidence](validation/2026-11-10-arch-clean-build.md). |
| P25 | Открытая Linux-поставка и release-приёмка | Частично | Приняты Rust inventory225/0missing, строгий диагностический RU/EN каталог и standalone export. User guides PR83 и exact ELF/Qt/Go composition PR85 ещё открыты на дату снимка. Нужны окончательные notices/source obligations, actual producer failure paths, evidence всех FR и integrated frozen package. Release/deploy не разрешены. [Каталог](validation/2026-11-11-p11-message-catalog.md), [приёмка](specs/validation-and-roadmap.md). |
| P26 | Android-приложение и поставка | Не начат | После P03: нативный UI/VpnService, встроенный WG, оба worker, protect/revoke/lifecycle/UID/package, полная матрица и измерения на физическом телефоне. [Платформы](specs/platforms.md), [этап 6](specs/validation-and-roadmap.md). |

## Правила дальнейшей работы

- Перед каждой задачей выбирать конкретный остаток и критерий завершения.
- Обычно 1–2 сабагента, только для независимых полезных частей; root интегрирует
  изменения и проверяет спорные выводы. Не повторять надёжное исследование целиком.
- Не менять критерий готовности ради повышения процента. При обоснованном изменении
  декомпозиции отдельно пересчитать знаменатель и объяснить изменение.
- После каждого закрытого пункта обновлять таблицу и итоговые числа. Partial
  превращается в «готов» только после необходимой runtime/acceptance-проверки.
- Пользовательские подписки/ключи сохранять; сетевые тесты проводить в собственном
  изолированном окружении. Секреты в repo, argv, логи и evidence не помещать.
