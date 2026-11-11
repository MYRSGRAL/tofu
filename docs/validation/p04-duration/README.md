# P04: campaign с ограничением времени и ресурсов

Linux, база `d0a281b4c23b45f65db1ddb74c6b14378b399794`. Android отложен.
Production parser не изменён. PR81 используется как corpus, его исследование не
повторяется. Этот срез закрывает измеренный duration/resource/canary gap, но не весь
P04: отсутствие panic при ограниченном отказе не означает поддержку всех валидных
входов. Coverage-guided fuzzing, sanitizers и формальная полнота остаются за рамками.

## Метод

Настоящий публичный `import_bytes`, release Rust 1.85.1, отдельный процесс на вход.
Бюджет 600 секунд выбран как нижняя граница требуемого диапазона 10–30 минут:
повторные обходы 29 семейств и разнообразные mutations, включая дорогие граничные
записи, вместо миллиона одинаковых ответов. PR92, compiler, snapshot tools, engine,
GUI, network, keyring не затронуты. Новых зависимостей нет.

Лимиты parser: 8 МиБ input, 32 МиБ decoded, 64 КиБ entry, 20 000 records, depth 64.
Controls: URI IPv4/IPv6/IDNA, Base64/Base64url, Xray unknown options с обязательным
review, sing-box opaque с сохранением native, размеры вокруг 64 КиБ/8 МиБ, depth и
число записей вокруг лимитов. Для decoded 32 МиБ отдельный достижимый stress case
не заявлен: внешний input ограничен 8 МиБ. После controls выполняются seeded
позиционные замены/вставки/удаления, включая синтаксис и неизвестные параметры.

На probe: адресное пространство 256 МиБ, CPU hard limit 2 с, wall 3 с, output 16 КиБ.
Supervisor использует nonblocking pipes, killpg и wait4. Negative controls: бесконечный
цикл для CPU и wall, выделение 512 МиБ, бесконечный вывод, раннее закрытие stdin,
дочерний процесс с удержанием stdout. Все шесть проверок прошли. MemoryError —
контролируемый отказ allocator, не доказательство kernel OOM. Необъяснённый SIGKILL
классифицируется как crash. RSS включает fork/pre-exec Python baseline.

Manifest записан до первого вызова; в нём source hashes, seed, budget и hash точного
бинарника. Внешний retained evidence:
`/home/myrs/projects/yonik/local/validation/p04-duration/`.
`final-600/before.json`, `final-600/cases.jsonl`, `final-600/summary.json`,
`bounded_campaign`, `Cargo.lock.used`, `replay-100/`.
Cargo 1.85 переупорядочил lockfile; проверено равенство версий/checksums и множеств
зависимостей, использованный lock сохранён отдельно. Репозиторный lock не входит в diff.

Первый короткий запуск остановлен: harness ошибочно требовал server review у opaque
sing-box (API возвращает native без servers). Это ошибка контрольного ожидания;
parser не исправлялся. Полный запуск выполнен с корректным opaque-контролем.
Replay индекса 100 совпал по hash, ответу, status, family и bytes. Сбой сохраняется
как синтетический `failure-INDEX.bin`; никакие raw user fixtures не используются.

Подробности повторения и ограничения чтения oversized prefix:
[tools/importcampaign](../../../tools/importcampaign/README.md).

## Дополнительные controls и наблюдение для root

`tools/importcampaign/boundaries.py` отдельно проверяет 14 крупных native/Base64
контролей на том же бинарнике, также с manifest до запуска. Все прошли: native opaque
побайтного размера 65 535/65 536/65 537, 6 МиБ, 8 МиБ−1/8 МиБ/8 МиБ+1 и их Base64
представления. Внешний размер определяет TooLarge; допустимый native остаётся opaque.
Максимум в этой серии: 0,0272 с, 60 612 КиБ RSS с fork baseline.

Отдельный минимальный probe для root (production не изменён):
`trojan://CANARY@` + `"a" * 64` + `.test:443` принимается как один server без review,
как и label длины 65; длина 63 также принимается. Команда построения синтетического
входа: `printf 'trojan://CANARY@%064d.test:443' 0` (ровно 64 цифры в label).
Это наблюдение о hostname validation; требуется решение root о требуемом контракте
DNS label, затем отдельный RED-тест владельца API. Никакой сетевой lookup не выполнялся.
Результаты находятся в `native-boundaries/results.json`. Смена API cooperative
cancellation принадлежит другому writer; campaign её не вызывает.

Снимок всех исходников из BEFORE manifest сохранён в `source-before/`, база и
параметры сборки — в `baseline.txt`; будущие изменения API не меняют измеренный
бинарник и не требуют перезапуска этой campaign.

## Окончательный результат

PASS ограниченного campaign-среза: 600.017 с, 93309 вызовов, 91838 уникальных входов, 29 семейств.
Parsed calls: 35637; fatal rejected calls: 57672.
Записи: accepted 141939230, rejected issues 64956, reviewed 2292 (подмножество accepted).
Timeout 0, memory failure/OOM observations 0, crash 0, output-limit 0, control/canary failures 0.
Максимум probe wall 0.040748 с; RSS 64252 КиБ, включая fork baseline.
Rust 1.85.1: fmt, clippy -p yonik-import --all-targets -- -D warnings, все тесты yonik-import прошли. Python: ruff check/format и 7 unittest (6 negative controls + сводка) прошли.
P04 целиком — НЕ PASS: сохраняются полнота форматов/семантики, coverage-guided fuzz/sanitizers и решение контракта DNS-label.

## Исправления независимого review

В исходном `final-600/summary.json` счётчик `rejected` смешивал 64 956 rejected
records и 57 672 rejected calls (122 628). Этот raw файл сохранён без изменения;
`analyze.py` и приведённые выше результаты уже разделяют обе величины. Новые
summary имеют `format_version: 2`, ключи `*_records` и `*_calls`. Регрессия
исполняет настоящий `campaign.run` с двумя контролируемыми ответами и проверяет
отдельные метрики. Для исправления сводки кампания повторно не запускалась.

Подстрока `memory` в произвольном stderr больше не означает отказ allocator.
Распознаются только завершённые диагностические строки `MemoryError` либо
`memory allocation of N bytes failed`; другие nonzero exits остаются crash.
Это классификация диагностического признака, не доказательство kernel OOM.
Negative control `memory unrelated` с exit1 воспроизводил прежний ложный статус
и теперь остаётся crash; настоящий Python MemoryError по-прежнему распознаётся.
Короткие controls подключены к стандартному `tools/tests` discovery; 10-минутная
кампания автоматически в CI не запускается.

RED/GREEN и проверки сохранены вне TTL:
`/home/myrs/projects/yonik/local/validation/p04-duration/review-fix-{red,green}.log`.
