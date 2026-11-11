# Интервал и фоновые окна подписок Linux

Policy реализации: интервал в целых секундах, минимум 86400, максимум u32::MAX
(ограниченное представление persisted задержки); legacy default — 86400.
Сохранённые due не сокращаются сменой настройки. Новый интервал используется
новыми автоматическими работами; retry budget/backoff остаются прежними.

Фон по умолчанию выключен. При явном разрешении открывается одно общее окно
на 60 секунд, когда наступил общий due и известна разрешающая network/power policy.
Следующее окно открывается не раньше настроенного интервала после предыдущего.
Окно не продлевается fetch/review/provider-событиями. При закрытии отменяется
незавершённый fetch с ожиданием существующего cancellation ACK; готовый review
остаётся до явного решения. Все источники используют общий deadline controller.
Unknown, metered и power saver не разрешают фон. Закрытие или запрет policy
сначала закрывает окно и подтверждает отмену, затем разрешено новое планирование.

Проверки: RED/GREEN для границ интервала, legacy/restart, общего окна и
непродления, отмены held fetch и сохранения review; DesktopHost/private bus
проверяется с barrier применённой policy. Физический GPOWER не заявляется.

## Основание и выполненные проверки

Acceptance взят из [performance §2](../specs/performance.md) и
[data-and-contracts §4](../specs/data-and-contracts.md): один scheduler,
настраиваемый автоматический интервал не менее суток, объединённое разрешённое
окно и событийные network/power ограничения. Число 60 секунд — согласованная
implementation policy Linux, а не число из спецификации.

Поведенческие RED независимо возвращали старое назначение суток, hardcoded
background=false и суточный reservation ручного fetch. Все три теста падали
на ожидаемых assertions; соответствующие GREEN прошли. Интервал сохраняется
также перед ручным запросом, без изменения refresh/cancellation/COMMIT API.

На Rust 1.85.1 с --locked прошли 97 тестов: 8 domain, 15 Linux scheduler unit,
6 Linux runtime, 32 desktop unit и 36 desktop backend. Один существующий backend
тест пропущен. Clippy всех targets трёх затронутых крейтов с явным pinned85 driver
и -D warnings — GREEN. Cargo.lock и зависимости не изменены.

Финальный отдельный DesktopHost/private-bus gate прошёл с barrier применённой
policy: hidden fetch, отмена held socket с closing ACK, persisted 48h/restart,
реальное истечение 60 секунд и сохранение review после закрытия. Готовый review
не продлевает окно. Supervisor также проверил timeout/reap собственной process
group и закрытие provider до публикации closed. Отсутствие запроса до завершения
GetAll/policy barrier не засчитывается как доказательство.

Evidence вне TTL: /home/myrs/projects/yonik/local/evidence/scheduler-windows/.
Перед запуском записаны source/tree/binary manifests; сохранены финальные
p10-gate и backend-test. GREEN logs: final-privatebus.log,
final-desktop-unit.log, final-desktop-backend.log, linux-unit.log,
linux-runtime.log, domain.log, clippy85-final.log. RED logs: red-interval.log,
red-window.log, red-manual.log. QML успешно разобран qmlformat; GUI interaction
и физические power/CPU/wakeup измерения не выполнялись. Полный GPOWER не заявляется.


## Замена runtime при смене Proxy lease

P2: прежняя replacement-ветка копировала только policy/visible, теряя настройки и монотонный next_window. Две unit-регрессии воспроизвели RED: потерю барьера после закрытого окна и потерю настроек/срока текущего окна. Новый DesktopSchedule::replacement переносит interval и настройку runtime, background_enabled, неизменные window_until/next_window, policy и visibility. Смена lease не продлевает общее окно и не позволяет открыть следующее раньше интервала. Новый runtime загружает актуальное persisted schedule после shutdown ACK старого.

GREEN Rust1.85.1: 34 library + 36 backend, один прежний ignored; pinned Clippy all-targets desktop/linux/subscription и fmt PASS. Это unit/direct-call regression replacement-метода и проверка production branch связи, не новый реальный Proxy lease traffic gate. Ранее сохранённый private-bus acceptance не переписывался. Evidence `/home/myrs/projects/yonik/local/validation/pr102-window-replacement/`; manifest записан после проверок.
