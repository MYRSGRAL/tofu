# Системная конфигурация TUN: план

Цель — связать управляемые TUN-адреса/маршруты и конфигурацию обоих закреплённых
ядер. Основание: P12/P13, networking.md и helper-owned-tun.md. Полная GUI/DNS
приёмка остаётся частью общего плана; этот этап не подменяет её.

- [x] Durable ResourceAddress с обязательным родительским TUN, валидацией,
  сохранением при reopen и отказом удалять родителя при оставшихся ресурсах.
- [x] Typed MTU/up/address/route операции: journal intent, exclusive ACK,
  точное наблюдение и обратный cleanup; неизвестный результат не присваивать.
  Обе семьи, чужие coarse-key совпадения, crash/reopen и ошибки.
- [x] Typed системный inbound компилятора обоих ядер: полученное от helper
  имя/MTU, без автоматического управления routes/DNS; сохранить политику,
  loopback probe и credentials. Проверить фактическое поведение address settings.
- [x] Через скомпилированный профиль пройти настоящий TUN→proxy→echo путь,
  config-check и две семьи/протоколы; direct echo не выдавать за proxy-проверку.
- [ ] Независимое review, исправления, локальные проверки и CI; обновить
  evidence и общий план, сохранив открытые остатки всей сессии.

Ограничения: Rust 1.85.1, без новых dependencies/unsafe; тесты только в своих
user/net/mount/pid namespaces. Основной agent: compiler/Journal schema и
интеграция; worker: закрытые сетевые эффекты. Секреты не сохранять в логах.

Обнаруженный неучтённый IPv6 эффект stock sing-box устранён явным pinned adapter
external_configuration; ordinary binary отклоняется до Apply.
[Evidence](../../validation/2026-11-10-system-tun-configuration.md).
