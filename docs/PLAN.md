# План завершения yonik

Начальная инвентаризация: 10 ноября 2026, UI `85b2a8e`. Текущие изменения отмечены ниже. Цель — пройти весь порядок из
[спецификации](specs/validation-and-roadmap.md): рабочий Linux-продукт,
проверенная поставка, затем Android. Пункты и критерии ниже уточняют порядок
исполнения, не заменяя требования FR и обязательные матрицы приёмки.

## Как считаем готовность

Всего **26 пунктов**: **3 готовы**, **21 частично выполнен**, **2 отложены (Android)**.
Остаётся полностью закрыть **23 пункта**. По числу полностью принятых пунктов:
**3 / 26 = 11,5%**. Это показатель закрытия критериев; задачи различаются по объёму.
Частичные результаты учитываются в описаниях, но не получают условные 50–90%.

Ранние G-LINUX/G-DNS/G-APP включены в соответствующие сетевые пункты, поэтому
одна интеграционная проверка не посчитана дважды. Android compile-check и
технический Android-host разделены: успешная компиляция не доказывает работу VPN.

Статус «готов» требует проверяемого результата по всему критерию пункта.
Исторические evidence подтверждают только описанные в них срезы. После правок
обязательны релевантные проверки; green CI и независимое ревью нужны перед
слиянием согласно действующим договорённостям. Нельзя закрыть весь план только
по наличию GUI, количества unit-тестов или namespace-прототипа.

## Актуальный acceptance-срез: main b9e5071, 11 ноября 2026

[Полный индекс Linux](validation/2026-11-11-linux-acceptance-index.md) связывает
все FR, P01…P26, gates, D/A/T, P-01…P-08 и leaves исходных контрактов с evidence
и остатком. Он имеет приоритет над исторической очередью ниже. Linux done не
заявлен; 3/26 (11,5%) сохраняется, P20 исправлен на частичный, Android P03/P26
отложен без изменения знаменателя.

Приняты #90 production CONFIG0600 12/12 на frozen 8ae373f (не #49 libtest),
#101 OFD, #114 native DNS partial, #107 6201cbe, #117 0ca6780, #118 b9e5071.
Открытые #109/#112/#115/#119–#123 учитываются отдельно от main; #91 остаётся
историческим открытым snapshot. Merge/CI/hash не заменяют runtime-приёмку.
Приоритет: integrated production Apply и domain/app provenance, полный DNS/private
lifecycle и итоговая поставка. По уточнению пользователя после #125 узкие
Go-интеграции обоих закреплённых ядер с Rust broker/TUN FD и локальный
`yonik-nsfs` adapter с Cargo edge явно разрешены; ожидание этих разрешений
больше не блокирует работу. Root возобновил #122 anchor; это не runtime proof.
Физические G-UI/G-POWER пользователь пока отложил: они остаются недоказанными,
без waiver или PASS; разблокировка KDE и hardware-counter permissions не заявлены.

## Историческая очередь (10 ноября; не текущий статус)

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
| P03 | Технический Android-host / G-ANDROID | Отложен | Оба engine через FFI, TUN FD/protect, TCP/UDP, stop/revoke, lifecycle и package callbacks; закреплённые wrappers. Остаток — runtime-host и измеренные результаты. [Контракт](specs/platforms.md). |
| P04 | Импорт URI/JSON/Base64 и ручной ввод | Частично | Есть ограниченный parser, сохранение исходника и review. Остаются все требуемые варианты ввода и табличные/property/fuzz-проверки размеров, вложенности, IDNA/IP и неизвестных опций. [Evidence](validation/2026-11-09-subscription-import.md), [тесты](../crates/yonik-import/tests). |
| P05 | Подписки, обновления и идентичность профилей | Частично | CAS, rename/delete, отмена, HWID и последний snapshot реализованы. Завершить proxy-загрузку без скрытого DIRECT и полную матрицу изменения credentials/пустого/частичного ответа. Плановые обновления — P10. [Snapshots](validation/2026-11-09-subscription-snapshots.md), [identity](validation/2026-11-09-profile-identities.md). |
| P06 | Зашифрованное хранение, миграции и восстановление | Частично | SQLite/Secret Service, миграции и CAS есть. Остаток — полная приёмка backup/recovery, сбоя транзакции, повреждения snapshot и закрытого keyring. Сохранить legacy data/key domains. [Код](../crates/yonik-linux/src/repository.rs), [identity](validation/2026-11-09-profile-identities.md). |
| P07 | Общая модель политик и семантика представлений | Частично | Доменная модель и приоритеты протестированы. Завершить полную матрицу AND/OR, стабильных ID, неподдерживаемых селекторов и сохранения семантики при преобразованиях; GUI редакторов — P20. [Код](../crates/yonik-domain/src/policy.rs), [evidence](validation/2026-11-09-policy-routing.md). |
| P08 | Capabilities и компиляторы обоих ядер | Частично | Есть config-check закреплённых ядер и явные отказы для части опций. Закрыть весь заявленный набор профилей/правил, включая ограничения смешанного DNS, без игнорирования параметров. [Evidence](validation/2026-11-09-engine-compiler.md), [routing](../crates/yonik-engine/src/routing.rs). |
| P09 | Lifecycle сессии и PreparedPlan | Частично | Reducer и поколения реализованы. Завершить host-интеграцию: конкурирующие Connect, отмена Validate/Apply, поздние события, повтор Stop, потеря host и подтверждённый cleanup. [Тесты](../crates/yonik-application/tests/session.rs), [evidence](validation/2026-11-09-prepared-plan.md). |
| P10 | Scheduler фоновых обновлений | Частично | Переносимая bounded модель с передаваемым временем, backoff, отменой и stale-защитой влита PR #44. [Evidence](validation/2026-11-10-subscription-scheduler.md). Нужны исполнение через SubscriptionService, подключение UI/политик сети и питания, фоновые окна и полная приёмка. [Приёмка](specs/validation-and-roadmap.md). |
| P11 | Ошибки и переводы RU/EN | Частично | Есть безопасная диагностика и переводы экранов. Завершить общий набор message-key, автоматическую проверку полноты обоих языков и отсутствие credentials/управляющих последовательностей в экспорте. [Контракты](specs/data-and-contracts.md), [GUI](validation/2026-11-09-native-desktop.md). |

## Linux-сеть

| ID | Пункт | Статус | Критерий полного завершения и текущий остаток |
| --- | --- | --- | --- |
| P12 | Privileged helper и владение ресурсами | Частично | Авторизация, [реестр](specs/helper-trusted-registry.md), [одноразовые artifacts](specs/helper-prepared-requests.md) и [бюджет](specs/helper-memory-budget.md) существуют. [Nftables executor](specs/helper-nftables-executor.md) и [правила блокировки](specs/helper-public-block.md) влиты PR #32/#33 с зелёным CI. [Durable policy schema v2/recovery](specs/helper-policy-recovery.md) проверены на шести SIGKILL-границах. [IPC v1, авторизованный Apply/Unlock и cgroup fence](specs/helper-service.md) реализованы и локально проверены с actual nft; сервис ещё не установлен. [Persistent TUN executor](specs/helper-owned-tun.md) локально проверен, но ещё не связан с session. [Адреса/маршруты и compiler](specs/helper-tun-configuration.md) проверены в namespace. [RPDB executor](specs/helper-tun-rpdb.md) проверен с mark bypass и cleanup. [Установленные worker units/cgroups](validation/2026-11-10-installed-worker.md) проверены в VM с реальными ядрами. Production non-app installed helper/system bus/polkit lifecycle подтверждён #90: 12/12 на frozen 8ae373f; остаток — live topology reconciliation, production app/domain Apply и полная матрица владения. См. актуальный индекс; ранние утверждения о wiring выше исторические. [Авторизация](validation/2026-11-09-helper-authorization.md), [journal](validation/2026-11-09-network-resource-journal.md). |
| P13 | Системный TUN обоих ядер / G-LINUX | Частично | Namespace TCP/UDP/IPv4/IPv6 пробы и persistent single-queue TUN для обоих ядер проверены; [owned TUN](validation/2026-11-10-owned-tun.md). Системный compiler и TUN→VLESS→echo проверены также через установленные systemd workers в одноразовой VM; sing-box требует pinned external-configuration adapter. Подключить helper/host к реальной политике, минимальным правам, исключению транспорта и stop/crash cleanup, пройти Linux-матрицу. [Evidence](validation/2026-11-09-tun-traffic.md). |
| P14 | DNS/Fake DNS / G-DNS | Частично | Есть модели, выявленные gaps и native pinning/provenance-патчи. Завершить единый fail-closed контракт D-01…D-07, поколения/TTL, оба ядра, Fake DNS on/off и IPv4/IPv6. [Native DNS](validation/2026-11-09-native-dns-contracts.md), [pinning](validation/2026-11-09-resolution-pin.md). |
| P15 | Атрибуция приложений / G-APP | Частично | Проверены отдельные executable-path пробы. Нужны реальная TUN-маршрутизация обоих ядер, одновременные приложения, составные правила, A-01…A-08, упаковки/ограничения и стоимость P-06. [Evidence](validation/2026-11-09-process-routing.md). |
| P16 | Приватные VPN, LAN и изменения сети | Частично | Read-only наблюдение topology/resolver реализовано. Нужны verified binding WG, split DNS, Apply и сохранение защиты при смене VPN/сети/sleep, затем корректное восстановление. [Наблюдение](validation/2026-11-09-network-observation.md), [topology](validation/2026-11-09-network-topology.md), [resolver](validation/2026-11-09-resolver-host.md). |
| P17 | ru-direct и комплексная Linux-приёмка | Частично | Policy-модель есть. Нужны обновляемые согласованные snapshots с rollback, реальные приоритеты и матрица двух ядер/Fake DNS, включая IPv6, MTU и сбои. [Evidence](validation/2026-11-09-policy-routing.md), [матрицы](specs/validation-and-roadmap.md). |

## Приложение и поставка

| ID | Пункт | Статус | Критерий полного завершения и текущий остаток |
| --- | --- | --- | --- |
| P18 | Последние согласованные UI-правки и HWID | Готов | yonik, узкое окно, card+gear, центральная кнопка; нет избранного, footer-кнопок/текста, бейджа и чекбокса анимации. Opt-in HWID стабилен и зашифрован. Установлена `85b2a8e`: по 142 GUI-действия в двух размерах, 6 health-сценариев, review без findings. [UI evidence](validation/2026-11-09-home-cleanup.md), [HWID](validation/2026-11-09-hwid-ui.md). Общая продуктовая приёмка — P19/P23. |
| P19 | Продуктовые экраны и системное подключение | Частично | Системный controller/GUI и preferences имеют component evidence; main merge не получил нового полного production GUI/TUN run. Нужны actual Apply, recovery, управление версиями и полная проверка пользовательских путей; OPEN #113/#115 не закрывают критерий. [Текущий host](validation/2026-11-09-native-desktop.md), [контракт](specs/ui-backend-contract.md). |
| P20 | Два редактора правил и каталог приложений | Частично | Редакторы, каталог и диагностика имеют component evidence; [L15](validation/2026-11-11-editor-policy-diagnostics.md). Осталось доказать атомарный actual Apply одного поколения и полную приёмку A-04/A-07/A-08 без потери семантики. [Спецификация](specs/routing-and-tray.md). |
| P21 | Трей и desktop lifecycle | Частично | Базовые команды и single instance есть. Пройти T-01…T-04 на KDE, restart Plasma, отсутствие трея, ошибки cleanup, закрытие окна и «Выйти и отключиться». [Evidence](validation/2026-11-09-native-desktop.md), [приёмка](specs/validation-and-roadmap.md). |
| P22 | Пакетные проверки серверов и диагностика | Частично | Есть HTTPS, три TCP-пробы и bounded API до 10 000 целей. Нужны пакетный GUI, endpoint/ошибки/ограничения и полная привязка результатов к поколению сети. [TCP series](validation/2026-11-09-tcp-series.md), [batch](validation/2026-11-09-tcp-batch.md). |
| P23 | Выбор toolkit, G-UI и G-POWER | Частично | Qt UI и отдельный стенд есть. Нужны реальные замеры Qt/GTK4/Slint в разблокированной сессии, продуктовая приёмка ввода/clipboard/масштабов/трея и P-01…P-08 с idle/hidden/attribution. [Стенд](../bench/ui/README.md), [бюджеты](specs/performance.md). Физические G-UI/G-POWER пока отложены пользователем, без waiver/PASS; критерии и статус частичного выполнения сохранены. Экран блокировки и питание без согласования не менять. |
| P24 | Arch-поставка, обновления и откат | Частично | Arch packaging реализован частично; OPEN #119 `6dc4658` содержит actual source build, old→new→old/uninstall, kernel upgrade и production userdata/future preferences. Нужны review/seal, интеграция и итоговая проверка поставки с trusted helper/ядрами; открытый delta не считается main. [Launcher](../tools/install_desktop.py), [платформы](specs/platforms.md). |
| P25 | Открытая Linux-поставка и release-приёмка | Частично | GPL и README RU/EN есть. Нужны notices/SPDX и аудит условий зависимостей, пользовательские инструкции, зелёные проверки, evidence всех FR и явные ограничения. Релиз/публикация выполняются только в разрешённом объёме. [Лицензия](../LICENSE), [приёмка](specs/validation-and-roadmap.md). |
| P26 | Android-приложение и поставка | Отложен | После P03: нативный UI/VpnService, встроенный WG, оба worker, protect/revoke/lifecycle/UID/package, полная матрица и измерения на физическом телефоне. [Платформы](specs/platforms.md), [этап 6](specs/validation-and-roadmap.md). |

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
