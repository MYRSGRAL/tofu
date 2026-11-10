# Управляемая RPDB системной TUN-сессии

Цель: заменить fixture `ip rule` реальным journal-backed executor перед
интеграцией worker/helper/controller. Продолжение P12/P13, не закрытие этих пунктов.

Архитектура: helper владеет отдельной routing table и правилом `not fwmark`
для каждой семьи. Помеченные сокеты ядра проходят существующую RPDB и не
попадают обратно в TUN. До полного host/reconcile контракт не применяется
к пользовательской сети; LAN, private VPN и DNS не получают фиктивной поддержки.

Стек: Rust 1.85.1, существующий RTNETLINK transport и durable Journal schema v2;
без новых зависимостей. [Исходный контракт](../../specs/helper-tun-configuration.md).

## Инварианты

- Только namespace-тесты, без установки/запуска системного helper на host.
- Полная метка TUN, ifindex и confirmed journal связывают rule с родителем.
- Фиксированы invert, полный mark mask и protocol 253. Никаких произвольных
  selectors, команд, replace или flush из IPC.
- До записи rule: exact default route нужной семьи и только собственные routes
  в выделенной таблице; коллизия priority с чужим rule запрещена.
- Intent durable до CREATE, ACK и exact dump до Confirmed. После неоднозначного
  create отсутствие доказательства владения запрещает adoption/удаление.
- Cleanup: rule → routes → addresses → TUN. Ошибка ребёнка сохраняет родителя.
- Существующие чужие правила, таблицы и объекты не удаляются.

## Задачи и доказательства

- [x] `managed_network/tun_rule.rs`: типизированный `TunRule`, ensure/remove,
  exact LinuxRule matcher с отказом при unknown fields; отдельный anchored
  `Resource::TunRule` и проверки документа/cleanup.
- [x] `tun_write.rs`: RTM_NEWRULE/DELRULE с CREATE|EXCL и строгим ACK.
  `tun_config.rs`: разрешить только confirmed owned rule references;
  запретить удаление route до rule.
- [x] `tun_rule_tests.rs`: corpus selectors/metadata, настоящие IPv4/IPv6
  route-get с mark/без mark, reopen, неизвестный intent, чужие priority/table
  references, обратный cleanup без изменения чужих объектов.
- [x] Canonical `tools/managed_network_smoke.py --case tun-rule`, CI namespace
  gate; не допускать ноль выбранных Rust-тестов как PASS.
- [x] Проверить workspace test/clippy/fmt, Python/Ruff и runtime namespaces;
  независимое ревью и исправления. [Evidence](../../validation/2026-11-10-system-tun-rpdb.md).
- [ ] Commit/PR и зелёный CI до merge.

## Следующая интеграция

Registry → immutable checked config → ограниченный worker → session host →
версионированный IPC → controller/GUI. До неё `system_tun` capability остаётся
false; успешная RPDB-проверка не доказывает готовность DNS/Fake-IP или Linux VPN.
