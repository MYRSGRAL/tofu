# ACK отменённой команды редактора

Review PR61 на f7b8067 выявил потерю ACK: Stop повышает generation, а queued
routing mutation старого поколения отбрасывалась молча. QML оставался в
routingPending до перезапуска. Все четыре mutations теперь получают matching
request_id с error=cancelled; состояние draft и revision не изменяются.
Существующий completeRouting снимает pending по этому ACK и сохраняет ввод.

Регрессия удерживает настоящий callback ready, принимает mutation, отправляет
Stop и освобождает callback. RED: ACK не пришёл за 10 секунд. После ACK тест
повторяет сохранение с revision=0, проверяя отсутствие скрытой записи и
возможность продолжить редактирование. Это backend interleaving; Qt-сценарий
именно этого interleaving не запускался.

В ветку обычным merge включён async provider shutdown из PR62. Исходный
Rust CI f7b8067 падал на p10_e2e.rs:412 (provider ещё подключён при closed).
Сырые логи: /home/myrs/projects/yonik/local/validation/p10-close-observation/.

Combined workspace: 593 passed, 0 failed, 24 ignored; отдельный P10 gate PASS.
Workspace clippy all-targets --locked -D warnings прошёл. Из PR62 также включён
02a2abf: ограниченное закрытие network monitor с cleanup failure outcome.
