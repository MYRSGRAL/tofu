# P11: replacement failure и интеграция A4

Исправление `eebb316aa2e25bb24a7a55007f154bee0ed54a05` закрывает два actual
callers, молча пропускавших ошибку `scheduled.replacement()`: смена proxy lease
и Stop. Общий приватный путь публикует typed cause. При initialize failure
новый owner ещё не выделен: generation остаётся null по strict V2 contract;
остальные причины сохраняют generation текущей операции.

Проверка на frozen base `cc66e4a`: реальный local allocator при MAX → Snapshot →
strict V2 export; два error slots с `scheduler_initialize_exhausted`.
Полная library: **118 PASS / 1 ignored**, Rust1.85.1. Удаление публикации причины
в source mutant даёт assertion RED `0 != 2`, exit101. Clippy all-targets/fmt PASS.
ELF/source сохранены и связаны до исполнения, pre/post485 inputs одинаковы;
[компактные bindings](p11-replacement-integration.json). Transient очищен,
активных ссылок нет. Это компонентный proof, не исполнение итогового merged ELF.

Промежуточные неуспешные попытки не скрыты: test callback требовал явного типа;
initialize ошибочно привязывался к ещё не существующему поколению; accidental
изменение старого assertion восстановлено до финальной сборки. Два parity-теста
при ручном запуске ELF сначала упали из-за неверного cwd, затем весь suite прошёл
из crate cwd. Superseded inputs/logs отдельно вне TTL; финальные bindings относятся
только к последнему проверенному source. Старые assertions не ослаблены.

R2 `932b7107` независимо проверил fix, source/ELF/log bindings и механические
union `7f7ee5f`/`964a873`. Все новые source updates записаны в candidate ledger,
исторические source/source_blobs/producer execution сохранены; прежние updates
перенесены в history. `P11_main_closed=false` остаётся историческим маркером,
как требует неизменённый gate. Это не новое присвоение historical runtime source.

Финальный source gate выполнен: 6 PASS / 0 FAIL / 0 SKIP. После CI/merge этой ветки original P11 становится Ready по prior criterion
join R1 `a8b6f83b` и narrow join R2 `932b7107`. Не устанавливается новый native
issuer/factory, runtime всех enum, physical UI или полная Linux-приёмка.

A4 интегрируется из `9337a518` (stacked merge #181) в accepted main `c079e9d`;
stacked base `2599fe70` уже ancestor main. Его инструменты/data/fixes сохраняются
побайтно, новых boot/extraction/runtime нет. R2 подтвердил exact union.
FullA4=false: live PID1 hooks и финальный source/package freeze остаются открытыми.

Предлагаемый PLAN после принятия: 8 Ready / 16 Partial / 2 Android deferred, всего26.
Исторические counts7/17/2 и более ранние срезы не переписаны. До CI/merge это
условие принятия ветки, а не заявление о текущем accepted main.

## Объединение с портами/протоколами

Feature #210 `b3792c9` получила R1 `188f726b` CLEAN и восемь зелёных CI jobs;
два Python jobs упали только на устаревшем P11 freshness binding. Вместо снятия
gate или отдельного обхода включён exact merge `f90f5bb1` без remerge delta.
Backend добавляет format5 save/native Unsupported и typed transport serialization;
Main — transport fields и четыре закрытых RU/EN validation strings. P11
replacement/error publication и существующий render/export сохраняются.
Effective source bindings обновлены на combined source, старые updates сохранены.
Это не новое исполнение historical binary. Финальный narrow recheck R2 `824cd624` на `fcc44e3` — CLEAN; CI итогового head обязателен до merge.
