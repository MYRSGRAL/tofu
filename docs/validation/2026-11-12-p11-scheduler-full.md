# P11: actual Scheduler Full в Linux GUI

Стенд `p11schedulerFullE2E` создаёт отдельный encrypted repository со случайным
ключом и сохраняет две валидную подписку через SubscriptionService. Production capacity остаётся 20 000. Закрытое поле HostConfig доступно только
через test-support ephemeral constructor с capacity в диапазоне 1..=20000.
Example выбирает 1. Capabilities, ABI, pins и W3 контракты не меняются. Нет positive context/state injection или fake producer.

Production DesktopHost загружает persisted subscriptions через production
DesktopSchedule и получает Full от настоящего Scheduler. Общий production Bridge
доставляет snapshot в Qt. `check_p11schedulerFullE2E.py` требует ready, total=2,
положительное scheduler_generation и два typed scheduler_full/scheduler_admission
контекста. QML interposer автоматизирует только UI. Текст ошибки берётся из Main.qml,
QML Button.clicked вызывает production copy_diagnostics_v2, который пишет native
clipboard; TextEdit.paste читает его. Каждый RU/EN
экспорт проходит независимый strict V2 validator.

Отрицательные controls изменяют копию actual snapshot: stale scheduler generation,
неизвестная причина, удалённые контексты, удалённый typed reason. Каждый отказ
обязан сохранить clipboard. Затем вызывается production shutdown; приёмка сохраняет closed snapshot и требует terminal Qt exit.

Evidence и сохранённый бинарник вне TTL:
`/home/myrs/projects/yonik/local/validation/p11-full-132/`.

Этот case не доказывает interval validation, owner/counter exhaustion, helper drain,
полный P11, visible compositor gate, Android или физические G-UI/G-POWER.

Проверено на Rust 1.85.1: GUI build PASS; GUI all-targets clippy с gui,test-support
и -D warnings PASS; backend test-support library tests 72/72 PASS; fmt check PASS.
Strict V2 Python tests 5/5 и factory coverage 2/2 PASS; Python syntax PASS.
Ruff отсутствует в PATH. Actual acceptance PASS: `acceptance-closed/` и
`acceptance-closed.log` (RU/EN exports, actual snapshot, closed snapshot, Qt trace).
Source trace: `source.sha256`, `source.patch`, `untracked-source.rs`, `harness.py`.
Binary SHA256: `b0c80021cd319f7952902d7e075a397bc8212d033429338e55b8a683a04e2350`.
Бинарник содержит build_id `0.1.0+792f1d985b06-dirty`; exact content привязан к
source.sha256. Проверка ссылок перед cleanup: []; cargo clean PASS, собственный
transient root удалён. Первый подход с 20001 sources отклонён repository limit
20000 до GUI; его лог сохранён как отрицательный результат, PASS ему не присвоен.
