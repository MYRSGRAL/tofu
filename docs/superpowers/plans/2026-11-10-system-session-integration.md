# Интеграция системной Linux-сессии

Цель: пользовательский Connect запускает проверенный системный worker и
управляемую TUN-сеть через авторизованный helper; Stop подтверждает завершение
worker и очистку. Этот план продолжает P09/P12/P13/P19 после TUN/RPDB executor.

Основание: [платформы](../../specs/platforms.md),
[IPC helper](../../specs/helper-service.md),
[компилятор TUN](../../specs/helper-tun-configuration.md).
Основной код — Rust; без новых Go-обвязок и изменения пользовательских ключей.

## Текущий срез

[Worker runtime](../../specs/helper-worker-runtime.md): launcher и supervisor
реализованы. В namespace проверен отдельный ненулевой UID, zero-capability
Check и NET_ADMIN-only Run обоих ядер через TUN/RPDB с peer за veth.
Private-bus tests подтверждают отмену, asynchronous jobs и lifecycle guards.
Пункты ниже остаются открытыми до полной приёмки: эти доказательства не
заменяют установленный systemd unit, cgroup tree и session host.

## Порядок интеграции

- [ ] Отдельный закреплённый systemd worker template, как предусмотрено
  platforms.md: выделенный системный пользователь, отдельная cgroup,
  фиксированный executable/argv/env, доступ только к конфигурации текущей
  сессии и `/dev/net/tun`. Определить и проверить минимальные capability обоих
  закреплённых ядер. `CAP_NET_ADMIN` остаётся сильной привилегией, не sandbox.
  `PrivateDevices=yes` не открывает TUN одним `DeviceAllow` — проверить видимость
  узла. Не менять действующий public-protection unit до готовности нового пути.
- [ ] Supervisor worker через systemd: старт фиксированного unit, подтверждение
  текущей invocation/cgroup, readiness, наблюдение exit, ограниченные TERM/KILL,
  подтверждение пустой cgroup. Только после этого разрешается удаление сети.
  `host/process::terminate` сейчас подтверждает PID, а не всё дерево; напрямую
  переносить LocalSession на привилегированный runtime нельзя.
- [ ] Реальный `SystemSessionHost` сериализует Prepare/Apply/Stop/recovery.
  Переиспользовать TrustedRegistry::snapshot, CheckedConfig::validate,
  PreparedRequests и pattern blocking supervisor из protection_host.rs.
  Version/config-check выполняются без runtime сетевых привилегий.
- [ ] Строго типизированный Prepare: только поддерживаемые поля сервера/политики,
  без путей, argv, среды, raw JSON, выбранных клиентом TUN ID/mark/table.
  Helper выбирает ресурсы и компилирует config; artifact связывает checked bytes,
  network plan, UID/unique sender, поколения, namespace и topology stamp.
  Учесть память всех компонентов в действующем общем бюджете.
- [ ] Apply повторно проверяет authorization/context/artifact, topology и
  endpoint escape, затем применяет защиту и сеть, запускает worker, сверяет
  TUN/адреса/RPDB, подтверждает actual TCP/UDP и только затем отдаёт Applied.
  Собственные Netlink events не должны ошибочно инвалидировать своё поколение.
- [ ] Stop/crash/restart: запретить новые эффекты, подтвердить отсутствие
  worker tree, очистить только доказанно свои ресурсы в обратном порядке.
  При включённом kill switch сохранить блокировку, при выключенном восстановить
  обычную сеть. Не выдавать CleanupConfirmed при неизвестном состоянии.
- [ ] Версионированный IPC и системный client/controller; capability system_tun
  включается только при работоспособности всего пути. Подключить GUI Connect,
  Stop, recovery/error и повторный запуск без потери подписок/ключей.

## Проверки перед приёмкой

Actual unit gate выполнять в одноразовой Linux VM, чтобы проверить systemd,
cgroup v2, установленный worker user, реальные unit-файлы, fork/setsid и
helper crash без установки helper на host. Локально доступны QEMU, xorriso,
SSH и чтение/запись /dev/kvm; запуск ещё не проверен. Образ брать только из
официального источника с проверкой целостности. VM, overlay, seed и временные
ключи — в собственном transient task-каталоге под root flock; evidence и нужные
артефакты сохранить вне TTL до остановки и очистки VM. VM gate не заменяет
полную приёмку приложения на CachyOS/KDE.

- [ ] Настоящие Xray/sing-box и compiler в отдельной сети: TCP/UDP, обе семьи,
  endpoint за veth, отсутствие зацикливания благодаря socket mark; отрицательный
  контроль без worker/peer и обратный cleanup.
- [ ] Подмена registry/config, лишние поля/пути, stale/consumed artifact,
  конкурирующие Connect, отмена Validate/Apply, поздние события и repeated Stop.
- [ ] Worker exit/fork/setsid, зависший потомок, helper SIGKILL/restart и cgroup
  fence: подтверждённое отсутствие старого worker до повторного Apply.
- [ ] Реальные минимальные права и systemd hardening обоих workers; готовность
  unit-файла или fake-bus тест не заменяет эту проверку.
- [ ] Изменение сети/приватный VPN/LAN и DNS/Fake-IP проходят отдельные матрицы.
  Пока они не доказаны, соответствующие возможности остаются unsupported.

Полный Linux-продукт требует также P14–P17/P20–P25. Этот план не меняет
знаменатель и не засчитывает частичную реализацию как закрытые пункты.
