# Подготовка закрытой guest fixture N5

Этот пакет содержит только проверяемый код подготовки DATA. Сейчас нет fresh
trusted guest base, immutable ELF/source receipts и разрешения на provisioning.
`manifest.pending.json` намеренно даёт `MISSING`. Никаких guest/VM, установок,
регистрации profile, сборок, queries, probes, SSH или загрузок здесь не выполнено.
Старый неуспешный A4 base не является допустимым fresh base и не переиспользуется.

## Интерфейс для runner

`fixture.py` — Python 3.12+, только стандартная библиотека. Модуль не содержит
subprocess, shell, сетевых клиентов, dispatch произвольных argv, FD или env API.

- `preflight(manifest) -> Validation`: строго проверяет схему и pins.
- `stage_inputs(manifest, inputs: Path, destination: Path) -> Validation`:
  копирует только фиксированные имена в **новый локальный DATA-каталог**;
  не устанавливает, не распаковывает и не исполняет файлы.
- `validate_receipt(manifest, receipt | None) -> Validation`: проверяет захваченные
  после setup DATA, source/kernel/profile bindings и installed contract.
- `setup_plan(manifest) -> tuple[dict, ...]`: возвращает **неисполняемые DATA**
  фиксированных `install` argv с timeout 10 секунд. При `MISSING` план пуст.
- `canonical(value) -> bytes`: JSON с sort_keys, ASCII и compact separators;
  SHA256 этих bytes — `manifest_sha256` для runner и будущего grant.
- `load(path)`: JSON не более 1 MiB; duplicate/unknown fields запрещены.

`Validation` имеет `status`, `reasons`, `manifest_sha256`. `READY` означает только
совместимость и полноту предоставленных DATA. Это не PASS runtime acceptance,
не аттестация чужого компьютера и не разрешение на регистрацию или исполнение.
`MISSING` означает отсутствующие pins/inputs/capture. Известное несовпадение,
неизвестное поле, неверная версия, float/bool вместо integer дают `REJECTED`.
Декодер ничего не понижает до legacy wire и не повторяет неуспешный setup.

## Manifest `SEALED_GUEST_FIXTURE_V1`

Точный набор top-level полей виден в `manifest.pending.json`. Ненулевые SHA256 —
64 lowercase hex, commit/kernel BUILDID — 40 lowercase hex. Null разрешён только
как явный отсутствующий pin и никогда не создаёт готовую fixture.

`source_commit`/`source_archive_sha256` связывают YonIK source с helper, worker,
desktop ELF. Каждая из пяти `artifacts` содержит собственный source commit,
source archive hash, ELF hash/size, build receipt hash, fixed installed path,
uid/gid/mode. Два prebuilt ядра имеют собственные upstream source archives и
receipts, а не фиктивную ссылку на YonIK source. Никакие Go builds не выполняются.
`SOURCE_BOUND_ELF_V1` receipt содержит строго `schema`, `source_commit`,
`source_archive_sha256`, `elf_sha256`; его **исходные bytes** также pinned.
Receipt — assertion доверенного builder, не самостоятельное доказательство,
что произвольный ELF действительно получен из указанного source.

Ожидаемые staged inputs: pinned `guest-base.img` (<=16 GiB),
`guest-base.receipt.json` (`FRESH_GUEST_BASE_V1`, trusted fresh origin,
NEW_ACCEPTANCE_N5 lineage, NEVER_RUN assertion доверенного builder), `source.tar`, `yonik-helper`, `yonik-worker`,
`yonik-desktop`, `sing-box`, `xray`, пять `<binary>.receipt.json`,
`sing-box.source.tar`, `xray.source.tar` и пять canonical unit files. ELF должен
быть ELF64 little endian x86_64, размером 64 bytes..128 MiB; source archive <=512
MiB; receipt <=4096 bytes; unit <=65536 bytes. Копирование потоковое, без
распаковки, с O_NOFOLLOW/regular-file/size и fstat до/после чтения. Частичный
каталог после ошибки не готов: seal `SEALED_GUEST_FIXTURE_V1.json` создаётся
последним. Повторное использование существующего destination запрещено.

`units` зафиксированы literal SHA256 source `3e3127d` (reviewed PR216 head).
`canonical-units.json` хранит те же **неисполняемые DATA** для pure-тестов и review;
никакие текущие systemd units не изменены. При изменении canonical units нужен
новый reviewed manifest/fixture-код, а не arbitrary override.

`profile` совпадает с точным Rust native sidecar schema 1: byte arrays SHA256/20
bytes kernel BUILDID, release, machine, ABI и policy id. Frozen coverage digest
`bd5d7474108a2a1e051734ea4c9097d43380341025ed9cd60e0f1442527d2983`
является только identity рассмотренной политики. Actual deployed worker ELF и
source hash связаны отдельно. Значения UNIT/synthetic не являются production
профилем; никакого profile в текущем host этот пакет не регистрирует.

`environment` pin-ит expected PID1 ELF, worker UID/GID, topology, UID/capabilities
и cgroup-controller captures свежей guest. Это inputs будущей acceptance, а не
наблюдения текущего host. Их нельзя получить подстановкой test defaults.

## After-setup receipt `SEALED_GUEST_RECEIPT_V1`

Receipt обязан быть создан доверенным collector **в OWN GUEST после setup**.
Он содержит manifest/base/provenance/source digests, setup-finished/captured
monotonic timestamps, actual kernel BUILDID/release/machine, installed files,
unit hashes, exact profile DATA и guest-only grant binding.

Для пяти ELF нужны actual hash/size/path/root uid+gid/0555 mode/regular kind и
полная ordered ancestor chain (parent до `/`, root-owned directories без
writable group/other bits). Native source/profile установлены root:root 0444
по `/usr/lib/yonik/native-{source-v1.tar,profile-v1.json}`; worker root:root 0555.
Profile file digest берётся от canonical JSON DATA. Таким образом digest policy
не подменяет installed ELF/source identity или readonly contract.

`pid1`: pid=1, uid=0, comm=systemd, actual executable digest и starttime.
`system_bus`: actual system-bus address, root UID, PID/start/socket inode и unique
owner. `identities`: helper root, exact worker UID/GID и pinned UID/caps capture.
`cgroup`: v2, canonical helper/worker paths, pinned controller closure.
`topology`: pinned full capture digest и actual net/pid/mount dev+inode identities.
Raw `capture` UTF-8 text (<=1 MiB) обязателен в identities/cgroup/topology;
его SHA256 сверяется с expected closure pin, одного echo digest недостаточно.
Эти closure digests collector обязан вычислять из retained actual captures;
повторение expected hashes без capture не является достаточным evidence.

Валидатор проверяет структурную согласованность receipts, а не происхождение
JSON от удалённой машины. Trusted fresh base/provenance, builder и collector —
явная TCB будущего пакета. Отсутствие trusted evidence остаётся `MISSING`; pure
модель не вправе записываться как actual receipt. Runner обязан удерживать raw
captures вместе с receipts и проверять provenance до интерпретации `READY`.

## Только будущий guest-only grant

Перед любым действием отдельно предъявляются: полный source commit/archive SHA,
SHA256 canonical **заполненного** manifest, base/provenance hash, пять ELF/build
receipt pins, exact kernel BUILDID и source/profile DATA, фиксированный setup
план и случаи runner. Разрешение должно прямо охватывать fresh OWN GUEST,
установку этих конкретных artifacts, guest-only profile registration и bounded
capture/acceptance. Текущая инструкция этого разрешения не предоставляет.

Будущий порядок: проверить trusted base и staged pins; проверить guest identity
и отсутствие UID/GID collision; исполнить только reviewed fixed install plan
в guest; отдельно разрешённым способом зарегистрировать pinned profile;
захватить actual facts **после** setup; проверить receipt; только затем допустить
отдельно разрешённые runner cases. Никакие отсутствующие PID1/bus/cgroup/topology
facts нельзя предполагать. Не устанавливать этот план на host. Создание VM,
скачивание base, изменение групп/пользователей, daemon-reload/start и runtime
probes не входят в `setup_plan` и требуют конкретизации будущего grant.

## Выполненные проверки подготовки

Пройдено 14 тестов `fixture_tests.py`; ruff format/check пройдены.
Только `fixture_tests.py`: pure synthetic DATA, собственные временные regular
files/symlinks, JSON/ELF-header модели. Ни ELF, ни unit, ни provisioning argv не
исполняются. Отдельно выполняются `ruff format` и `ruff check`; новые зависимости
не устанавливаются. Реальных acceptance receipts в этом каталоге нет.
