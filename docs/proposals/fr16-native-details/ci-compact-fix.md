# PR196: compact diagnostics scroll regression

Frozen failure: `6aec304d5b9e20465f08b03be12c9e6a4f0aab8d`.
CI desktop run43559228035/job132338963813: normal420×760/scale1 проходит142 actions;
compact380×600/scale1.5 останавливается на step78 через45s (Qt2/app exit1).
Полный CI log и downloaded `ui-a-failure` artifacts сохранены вне TTL:
`/home/myrs/projects/yonik/local/evidence/fr16-ci-6aec304`.
Предыдущий55c9008 имел оба workflow SUCCESS (43556879704/43556875526), оба6aec304 —
FAILED (43559228035/43559223821). Это не объявлено прежним flaky timeout.

## Причина и RED

Step77 отправляет единственный wheel -1200 в diagnosticsScroll. Step78 ждёт
`atEnd=true,moving=false`, затем отправляет обратный wheel +1200. В FR16 добавлен
постоянно видимый многострочный withholding notice до ListView. Он увеличил высоту
ещё не прочитанного журнала: в compact layout одной прежней порции scroll больше
не хватает до конца. Это deterministic layout regression, а не медленный backend.

На сохранённом feature ELF с тем же production UI исходный compact script повторил
**RED на том же step78**. Диагностический проход отдельно подтвердил
`atEnd=false,moving=false` после первого wheel; второй downward wheel достиг конца.
Симметричный дополнительный upward wheel позволил завершить144-action diagnostic.
Deadline не менялся. Это диагностические дополнительные действия, не final fix.
Скриншоты и trace сохранены; classifiers/lifetime не переаудировались.

## Узкая правка

LogsView notice виден только для актуальной явно прочитанной страницы с ненулевым
rejected counter: `page && !stale && rejected !== "0"`. До чтения/для stale или
нулевого counter пояснение не сообщает о результате и не занимает место.
Положительный native witness проверяет, что нужное пояснение **не скрыто навсегда**.
Добавлен `logsWithheldNotice` objectName для focused assertions.

`tools/desktop_ui_smoke.py`, исходные142 actions, wheel delta, atEnd/atStart и45s
deadline не изменены. Нет изменения backend, native producer, worker/wire,
Rust classifier/reader/disk, deps/ABI/pins или N5 scope.

## GREEN и limits

- Rust1.85.1 GUI binary build — offline/locked, shared Rust slot, task-root flock,
  jobs1 и пустые wrappers. Новый ELF/source binding вне TTL.
- `fr16_notice.py`: тот же142-action сценарий плюс более строгий predicate notice
  hidden перед прежним wheel. **Оба layout GREEN**:420×760/1 и380×600/1.5; прежние
 10 negative-script cases, TCP/HWID/cancel checks и screenshots также пройдены.
- `fr16_local_native.py`: оба pinned core LocalSocks Start → native body UI/export/
 copy/save → Stop GREEN; дополнительно notice visible при actual rejected messages.
 Worker не участвовал, health/peer — прежние owned same-UID loopback fixtures.
- Ruff check/format и diff checks PASS. Rust source не изменён, classifier/lifetime
 review758868 принят как предоставленный Root scoped CLEAN; новый full review не запускался.

CI6aec artifacts и прежний ELF/source evidence не переписаны. Local GREEN получен
на сохранённом environment Qt; remote CI нового head требует отдельного результата,
его PASS здесь не заявляется. Это compact UI fix, не full FR16 PASS.

`ci-compact-evidence.json` связывает frozen/diagnostic/fixed results и hashes.
Собственный Cargo target очищен штатным cargo clean после сохранения ELF,
source snapshots, результатов и проверки отсутствия активных references; task root удалён.
