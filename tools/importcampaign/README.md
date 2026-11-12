# Ограниченная campaign действующего import API

Linux, Python 3.12+, Rust 1.85.1. Зависимости не добавляются. Сборка только через
`local/orchestration/with-rust-slot`, `CARGO_BUILD_JOBS=1`; target следует разместить
в собственном transient-каталоге, удерживая BSD flock его корня. Сохранить release
`bounded_campaign` вне TTL перед очисткой Cargo.

```sh
python -m unittest discover -s tools/importcampaign/tests -v
python tools/importcampaign/campaign.py /absolute/path/bounded_campaign /absolute/path/new-evidence --seconds 600
python tools/importcampaign/campaign.py /absolute/path/bounded_campaign /absolute/path/new-replay --seconds 1 --replay 100
```

Seed по умолчанию `0x5040465230372026`; replay воспроизводит RNG до указанного
индекса, включая все операции предшествующих мутаций. Для точного повторения нужны
те же source hashes и бинарник из `before.json`. Входы синтетические, из corpus PR81;
URI/Base64/Base64url, Xray и sing-box opaque, IDNA/IP, размеры 64 КиБ и 8 МиБ,
глубина и количество записей вокруг лимитов. После обязательных controls выполняются
1–16 замен/вставок/удалений ASCII-разделителей в различных позициях. Это mutation
campaign с бюджетом времени, без coverage guidance, sanitizer и новых fuzz crates.

Supervisor: на каждый процесс wall 3 с, CPU 2 с, адресное пространство 256 МиБ,
вывод 16 КиБ; неблокирующие pipes, kill process group и wait4 в finally. CPU hard
limit возвращается как crash со signal 9; `memory_failure` означает диагностированный
отказ allocator, а не доказанный kernel/cgroup OOM. SIGKILL без диагностики остаётся
crash: причина не угадывается. `ru_maxrss` включает Python fork/pre-exec baseline,
поэтому это верхняя оценка RSS процесса, а не чистая стоимость parser. Parent не
ограничен этими rlimits; сохраняется только ограниченный вывод дочернего процесса.

`accepted/rejected/reviewed` в summary — числа записей (reviewed является подмножеством
accepted). `parsed/rejected` также учитывают исходы вызовов: rejected включает фатальные
отказы. JSONL позволяет отличить их по `output` и разобрать каждый family отдельно.
Результат с нулём servers допустим для opaque native JSON; такой контроль проверяет
наличие сохранённого native. URI/Xray с неизвестными параметрами обязаны иметь review.

Probe проверяет сохранение оригинала при успехе/отказе и отсутствие CANARY в Debug.
Он читает максимум MAX_INPUT_BYTES+2: более длинные мутированные входы исследуют
ограниченный префикс, а не передачу всего oversized input в API. Начальные точные
границы вплоть до MAX_INPUT_BYTES+1 передаются полностью. Нет сетевых обращений,
keyring, engine или compiler; opaque native не выполняется и не считается поддержкой
семантики sing-box. Неизвестный вход может корректно отклоняться. NoPanic и bounded
отказ не доказывают поддержку всех валидных входов, отсутствие дефектов или полный P04.

При сбое сохраняются `failure-INDEX.bin` и JSONL с hash/index/family. Replay любого
индекса применим и без аварии. `before.json` записывается до первого вызова. Полная
campaign ограничена заданным временем плюс завершение текущего probe (до 3 с);
начальные controls выполняются обязательно даже при коротком smoke-бюджете.

Крупные native/Base64 controls на том же бинарнике:

```sh
python tools/importcampaign/boundaries.py /absolute/path/bounded_campaign /absolute/path/new-native-evidence
```

Этот короткий дополнительный набор пишет собственный BEFORE manifest. Контроль
DNS-label в нём является наблюдением для согласования контракта с владельцем API.

Сводка разделяет фатальные отказы вызовов и rejected records:

```sh
python tools/importcampaign/analyze.py /absolute/path/evidence/cases.jsonl
```
