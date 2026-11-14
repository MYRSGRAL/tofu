# A-04: минимальный контракт destination port / transport protocol

Предложение для root review, не утверждённая семантика и не реализация.
Дополняет structural finding `2026-11-14-p07-p20-port-api-gap.md`.
Domain/wire/schema/capabilities этим документом не изменяются.

## Предлагаемый минимальный scope

Только destination transport port и transport protocol TCP/UDP. Не process name,
PID, source port, ICMP type/code, application protocol, TLS/SNI или новые authority.
Порт относится к исходному назначению потока до outbound/proxy rewriting, а не
к адресу SOCKS/ядра/туннеля. UDP включает QUIC как UDP, но не распознаёт QUIC.

Rule condition концептуально состоит из optional `transport_protocols` и optional
`destination_ports`. Отсутствующая группа означает отсутствие ограничения этой
группы, как старые правила. Заданная группа обязана быть непустой; пустое значение
не должно превращаться в «любой». Для GUI очистка поля удаляет группу явно.

- Protocols: закрытое множество `tcp`, `udp`; OR внутри, без повторов.
  Unknown names/numeric protocols/`any` не принимаются.
- Ports: максимум 16 непересекающихся включительных диапазонов на правило,
  каждый `1 <= first <= last <= 65535`. Одиночный порт — вырожденный диапазон.
  Ввод валидируется до сохранения; parser сортирует, объединяет соседние диапазоны,
  отвергает пересечения/повторы. Лимит проверяется до и после нормализации.
  Transport port 0 не предлагается как поддержанный destination.
- Protocols OR, ranges OR; между executable, platform selector, domain,
  destination CIDR, protocols и ports — AND. Общий first-match порядок,
  private/VPN/LAN приоритеты, default action и ID не меняются.
- Только ports без protocols означает TCP ИЛИ UDP с указанным destination port.
  Только protocols не ограничивает номер порта. Без обеих групп — старое поведение,
  включая прочие потоки, которые модель раньше могла обработать.

Проверяемый пример: app=A AND domain=example.org AND CIDR=192.0.2.0/24 AND
protocol=tcp AND port=443 → Block, default Proxy. TCP/443 совпадает; TCP/8443 и
UDP/443 не совпадают; они проходят к следующим правилам. Если transport metadata
недоступна или не подтверждена, это НЕ доказанный non-match и не разрешение
перейти к более широкому default.

## Flow metadata и fail-closed

Предлагается закрытый transport witness из production adapter: KnownTcp(port),
KnownUdp(port), KnownOther или Unavailable. GUI/caller не создаёт provenance и
не задаёт metadata вместо adapter. `KnownOther` заведомо не удовлетворяет
TCP/UDP/port-ограничению. `Unavailable` при наличии применимого transport-условия
даёт явный Unsupported/Unknown без fallback на разрешающее правило.
До DNS lookup дешёвые отрицательные executable/protocol/port проверки могут
отсечь правило. Неизвестная transport metadata не должна запускать ненужный
DNS lookup ради фиктивного подтверждения правила.

Admission компилятора/adapter должен заранее отвергать политику с enabled
transport conditions, если не может обеспечить исходный destination/transport
witness и всю семантику. Отказ до изменения active generation; текущие policy,
revision и runtime остаются прежними. Disabled conditions сохраняются как данные.
Наличие capability/успешный compile само по себе не доказательство enforcement.

## Совместимость, которую root должен утвердить до кода

1. Явная новая версия Draft/wire; старые readers обязаны отказать до write/Apply,
   если поля могут потеряться. Нельзя добавить их под старой версией и silently
   ignore. Старые payload без полей читаются без изменения их поведения.
2. Storage migration/barrier должна сохранять все условия disabled и enabled
   правил, порядок/ID/default и CAS. Номер формата и необходимость verified
   backup выбираются после проверки всех reader paths; здесь не назначены.
3. Simple/Advanced load/edit/duplicate/order/Save сохраняют скрытые conditions.
   UI показывает наличие составного правила; обычное редактирование app/domain
   не очищает port/protocol. Save unsupported draft разрешён; activation запрещена.
4. Wire/native adapters должны передать точный transport witness; отсутствие
   поддержки в одном ядре даёт Unsupported для него, без broadening правила.
   Новые capability bits/ABI numbers требуют отдельного решения root.

Предлагаемые бюджеты: 2 protocols, 16 ranges/rule и 4096 ranges/policy при текущем
максимуме 256 rules. Существующий raw-byte лимит Draft сохраняется; не расширять
его неявно. Parsing O(n log n) на максимум 16 ranges; matching O(n), без DNS/I/O
и без дополнительных аллокаций на поток.

## Минимальная acceptance после утверждения

Parser boundaries/unknown/empty/duplicates/limits; AND/OR и false-source/no-lookup;
443/8443 TCP/UDP/other/unavailable; private/LAN/user priorities; disabled persistence;
Simple↔Advanced/order/Save/fresh reopen full equality; old-reader refusal и CAS
rollback; actual activation failure без потери active generation; затем отдельный
source-bound native witness на применимых ядрах. Никакой offline Save или fake
caller metadata не заменяет последний пункт. Старые потоки не объявляются
переклассифицированными без отдельного session lifecycle контракта.

Root должен явно утвердить: destination-only scope; диапазон 1..65535;
TCP/UDP-only с fail-closed Unavailable; нормализацию; version/barrier strategy;
capability/ABI strategy. До этого production изменения запрещены scope задачи.
