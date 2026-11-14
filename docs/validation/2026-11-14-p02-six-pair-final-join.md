# P02 SHORT: qualified six-pair join

**Partial**. R2 `14f72ca8` qualified CLEAN принят как trusted handoff: 297 pins
и семь исходных artifacts. DATA-only replay: пять старых и одна новая pair,
шесть пар, 12 окон, 372 retained sample points. Новых измерений при интеграции нет.

[Обработанный отчёт](../evidence/p02-short-private-20261114/six-pair-final-join/report.md),
[comparison](../evidence/p02-short-private-20261114/six-pair-final-join/comparison.json),
[verification](../evidence/p02-short-private-20261114/six-pair-final-join/verification.json),
[integration receipt](../evidence/p02-short-private-20261114/six-pair-final-join/integration-receipt.json).

Медианы full−engine: sing-box PSS 97518 KiB, Private_Dirty 80832 KiB,
CPU −0.01 s; xray 88759 KiB, 59916 KiB, +0.02 s соответственно.
USS и wakeups неизвестны (`null`). Near-zero CPU не доказывает регрессию или
экономию. P02, physical GPU/energy и финальный 8h baseline остаются Partial.

Старая noisy SB2 pair (overlap 148.3366525 s) исключена целиком, не удалена.
Пять старых пар используются observationally: source guard backport не заявлен,
новое выполнение guards для них не доказано. Исторические measurement source
`35408d531a86ec858e1d2fc3489e8e092fda5d27` и posthoc driver
`51fd15a3a94c1e5e2ca400e1dcd7ccf26438715e` сохранены.
Новый selector source `b1b9ce10525082039ecbdf87a6abbf33fc11449f` связан с тем же
reviewed driver. В каждой паре одинаковы config bytes и core ELF, source bindings
сохранены; sampler вне product group. Новый terminal receipt: exit 0,
восемь owned PID отсутствуют, cleanup fallback нет. Это сохранённые факты,
не новые host probes. R1 подтверждает только шесть pure/mock selector tests
и read-only closure; full native и whole-head validation не доказаны.

Accepted main `a3f609e66e5a95b978c0bf3da34ade4f1445e219` включён ordinary merge.
Product blobs и incoming P11 metadata сохранены; parents/tree/P11 blob receipt
находятся в integration receipt (`new_runtime=false`). Raw secret configs,
ELF и большие логи в этот перенос не входят.

Для DATA-only replay при наличии локальных pinned inputs:

```sh
cd docs/evidence/p02-short-private-20261114/six-pair-final-join
python3 -B verify.py
sha256sum -c artifacts.sha256
```

`source-artifacts.sha256` — неизменный исходный receipt семи artifacts;
`artifacts.sha256` — receipt перенесённых файлов. Reproducer scripts отформатированы
Ruff 0.17.4, lambda в report заменена на def; comparison/report bytes неизменны.
Архивная строка Ruff в verification относится к исходному join; при интеграции
Ruff check и format прошли. Чистые selector tests: 6/6; replay: 12 окон,
297 pins и пять отрицательных случаев. Rust/native/GUI/build не запускались.
