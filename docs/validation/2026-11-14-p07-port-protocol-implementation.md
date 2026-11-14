# Destination TCP/UDP: реализация и границы доказательства

Принятое root решение реализовано поверх cc66e4a: исходный destination TCP/UDP,
порты 1–65535, максимум 16 диапазонов/rule и 4096/policy. Группы соединяются AND,
значения внутри группы — OR. Диапазоны сортируются, соседние объединяются;
пустые группы, пересечения, повторы и неизвестные протоколы отвергаются.

TransportWitness различает TCP, UDP, KnownOther и Unavailable. Старые callers
передают Unavailable через прежний API. Применимое transport-условие при
Unavailable отвергает оценку до DNS и не пропускает поток к default. Проверки
источника и известного отрицательного transport результата не требуют DNS.
Private/LAN порядок не изменён. Новых native callers и authority нет.

Draft/wire v3 сохраняет transport как закрытый объект `transport` с optional
`transport_protocols` и `destination_ports` (массив пар включительных границ).
Legacy v1/v2 читаются; transport в legacy версии запрещён. Wire depth для v3
ограничен 6 из-за массива пар; v1/v2 сохраняют depth=5 и прежние byte limits.
Редактор сохраняет скрытые условия при app/domain edits, дублировании и reorder;
очистка GUI явно удаляет группы внутри transport. Введены bounded поля и RU/EN
сообщения ошибок. Port-only правило не требует подтверждения broad rule.

Первый v3 Save требует verified current backup format3/4. Проверяются версия,
identity и ciphertext всех таблиц; старый backup3 не подтверждает изменённую
format4 БД. CAS, ciphertext и format5 коммитятся одной транзакцией. Fault/stale
CAS оставляют прежние revision/payload/format. Все прежние format4 writers
сохраняют format5. Старый reader базы отвергает user_version>4 до write;
это подтверждено исходным guard, отдельный старый GUI ELF не запускался.
Тест future-version использует теперь 6, а неизвестного Draft/wire — 4: прежний
критерий отказа сохранён при введении принятых версий 5/3.

Enabled transport отвергается обоими compilers, resource-bound validation,
inline conversion и ранним normal Host connect gate. Disabled transport хранится.
Отказ происходит до native activation. Actual native enforcement **OPEN**:
новые capability bits/ABI, TUN, kernel probes, VM и adapter witness не добавлены.
Активная существующая native session отдельно не воспроизводилась.

[Proof](p07-transport/proof.json) связывает retained ELF, source archive и логи.
GUI campaign `tools/p07_transport_persistence.py` использует production Main.qml,
bridge и normal DesktopHost, отдельные DBus/Secret Service и software renderer.
Три процесса подтверждают exact snapshot equality через mode/order/Save/reopen,
сохранение enabled/disabled условий и unsupported connect. Это persistence proof,
а не enforcement. Owned GUI/Secret Service процессы закрыты. ELF/source/logs
сохранены вне TTL в указанном proof каталоге. После GUI добавлена только таблица
literal flow test; production closure осталась побайтово неизменной.

Проверки: Rust 1.85.1, locked/offline/jobs1; domain/engine tests PASS, editor 14/14,
linux unit 575 PASS/38 existing ignored; all-targets clippy четырёх crates PASS
с -D warnings. Final transport suite 4/4 и domain clippy PASS. GCC16/Qt C++ warnings
сохранены в build log. Ruff отсутствует, новые зависимости не устанавливались.
