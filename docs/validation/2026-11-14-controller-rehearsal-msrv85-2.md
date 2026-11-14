# A4: второй nonfinal rehearsal с installed MSRV profile

Root выбрал новый ID `linux-controller-rehearsal-14229843-msrv85-2` и тот же
frozen product source `14229843f9a11bf98ec4f5992df00f3247102ea2`.
Это не final freeze: `final_linux_acceptance=false`, factory/N5 и production
admission остаются закрыты. Runtime16, probes, bootstrap и cross-UID не запускались.

## Ограниченное исправление controller profile

В первом rehearsal `build_env` отбрасывал `RUSTUP_TOOLCHAIN`, наследуя PATH
с неисправным default rustup proxy. Два новых теста сначала дали RED:
неверный PATH и отсутствие `controlled_env` в command record.

Controller теперь сам выбирает фиксированный installed profile
`1.85.1-x86_64-unknown-linux-gnu`: его bin стоит первым в PATH, `RUSTC` указывает
на exact binary, `RUSTUP_TOOLCHAIN` задан controller. Candidate CLI не получил
параметров source, profile, record или command. Сохраняется фильтрация caller
RUST/CARGO/GO/GIT/YONIK overrides, jobs=1 и штатный `with-rust-slot`.

`LocalRunner` пишет effective allowlisted build environment в каждый command
record, включая неуспешные команды. Успешный collected context также содержит
эти значения и фактические версии rustc/cargo. Произвольное окружение и секреты
в record не копируются. Freeze связывает изменённое tooling по SHA, frozen recipe
и product archive не меняются. Root TCB и fixed registry не изменены.

## Проверки

14 controller-тестов и 16 provenance-тестов проходят. RED/GREEN логи сохранены
в [evidence](../evidence/linux-controller-rehearsal-14229843-msrv85-2/).
`ruff format --check` и `ruff check` проходят для двух изменённых Python-файлов;
использован уже установленный ruff из uv-cache, без установки зависимостей.

Прежний registry ID проверен по сохранённому SHA inventory: все records и
артефакты неизменны. PR175/171/147 и FR07 refs из первого отчёта сохраняются.

## Фактический результат: BLOCKED после двух успешных Rust builds

Операции выполнены штатным CLI из этого worktree, без candidate attestation:

```sh
python tools/release_controller.py freeze --release-id linux-controller-rehearsal-14229843-msrv85-2 --commit 14229843f9a11bf98ec4f5992df00f3247102ea2
python tools/release_controller.py ingest-cores --release-id linux-controller-rehearsal-14229843-msrv85-2
python tools/release_controller.py collect --release-id linux-controller-rehearsal-14229843-msrv85-2
python tools/check_package_provenance.py --release-id linux-controller-rehearsal-14229843-msrv85-2 --package /home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-2/package.tar --inputs /home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-2
```

Exit: **0, 0, 2, 2**. Фактические версии: rustc 1.85.1 (4eb161250),
cargo 1.85.1 (d73d2caf9). GUI release build прошёл за 8м36с,
helper/worker release build — за 2м58с. Обе команды прошли через slot wrapper,
`CARGO_BUILD_JOBS=1`, target находится в собственном transient task.
GUI CXX/Qt headers дали warnings; ошибки компиляции/линковки отсутствуют.

Три Rust ELF сохранены самим controller в canonical registry:
`/home/myrs/projects/yonik/local/release-controller/linux-controller-rehearsal-14229843-msrv85-2`.
Там же находятся frozen archive, accepted-cores с обоими Go ELF, manifests,
prebuild records, trusted-core-job и controller logs. Все пять файлов прошли
статический ELF64 x86_64 header check; пути, размеры и SHA — в `actual-elfs.json`.
Ingest связывает accepted component job с frozen pins/patches и не объявляет
его Go VCS metadata новым product SHA. Полная последующая проверка обоих cores
не завершена: collect дошёл только до xray.

Xray source реконструирован штатным controller на pinned
`a4a3dff5b20c2faae567b742c172c7b95e75c083`; native patch check/apply и
`go version -m` завершились с exit 0. Затем `check_core` вызвал `source_offers`
и получил **«Некорректный frozen Go source offer»**.

Конкретный новый finding: `source_offers` индексирует записи только по имени
module и отвергает второе вхождение, хотя frozen inventory содержит разные
версии одного module для разных cores. Первый конфликт —
`github.com/andybalholm/brotli`: index 4 / v1.1.0 и index 5 / v1.2.3,
с разными SHA. RED на настоящем frozen inventory и все конфликты сохранены
в `source-offers-red.json`. Это не отсутствующий файл и не toolchain error.
Исправление требует согласованного version-aware binding source offers к
Go BuildInfo/dependencies во всех потребителях; простое разрешение duplicate
или выбор последнего значения ослабит provenance. Это отдельный root proposal;
в этой итерации binding не менялся и collect не повторялся.

`collected.json` и candidate package не созданы. Candidate gate вернул exit 2:
`Нет controller-collected build`. Package compliance и положительный provenance
**не заявляются**. Ограничение двух Rust builds исчерпано этим запуском.

## Сохранение и очистка

Команда controller `cargo clean` завершилась с exit 0: удалено 4064 файла,
1019.4 MiB. До удаления остатков task сохранены ELF в registry, source/tree,
команды/exit/effective env, manifests и hashes. Вне TTL дополнительно оставлены
GUI backup и patched xray source archive:
`/home/myrs/projects/yonik/local/evidence/linux-controller-rehearsal-14229843-msrv85-2`.
Это наблюдаемые резервные артефакты, а не вручную внесённые trusted records.
`preserved-source.json` фиксирует путь и SHA архива.

После проверки отсутствия target и ссылок процессов на task выполнена очистка
с exclusive flock самого корня; собственный transient каталог удалён.
`failure.json.left_task` отражает путь на момент ошибки, актуальная очистка —
в `cleanup.json`. Старый ID и его failure records не переписывались.
Будущий final build после N5 product changes остаётся отдельной задачей.
