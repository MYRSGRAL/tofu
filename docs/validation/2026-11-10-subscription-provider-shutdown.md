# Provider освобождается до closed

База: 9933c94f10a8934a74e38b07a62ae243a0316ff3.
В PR61 полный desktop suite наблюдал ListNames=3 вместо 1 после shutdown.
Изолированный p10 gate на чистой базе прошёл; исходный единичный failure
сам по себе не доказывает независимость от изменений редактора.

По коду provider future оставался жив до выхода из run, хотя callback closed
и Control::closed вызывались раньше. Детерминированная регрессия удерживает
callback closed, проверяет фактическое исчезновение клиентских соединений
на private bus, затем освобождает callback и завершает cleanup. RED на базе
не получил исчезновения соединений за 3 секунды. Это воспроизводимый базовый
дефект порядка освобождения, а не воспроизведение каждого условия исходного
падения полного suite редактора.

Provider теперь имеет отдельное владение Box::pin и явно уничтожается до
shutdown monitor, закрытия repository и публикации closed. Нельзя просто
drop прежней Pin-ссылки tokio::pin: это не уничтожило бы future на стеке.
Admission policy, сетевые запросы и cancellation scheduler не менялись.

Daemon обрабатывает закрытие сокетов асинхронно. Gate ждёт реального ListNames=1
с общим deadline 3 секунды и интервалом 10 мс; истечение deadline остаётся FAIL.
Новый regression проверяет результат при удерживаемом closed callback, поэтому
поздний выход из run больше не маскирует живой provider.

GREEN: полный cargo +1.85.1 test -p yonik-desktop --locked и отдельный gate
прошли. БД/ключи/DBus эфемерные; host bus/keyring/network не менялись.
Qt/Wayland/энергоприёмка этим изменением не подтверждены.
Сырые логи вне git: /home/myrs/projects/yonik/local/validation/p10-close-observation/.
Cargo target в собственном transient root под flock, jobs=2.

Финально: 46 desktop tests passed, 1 ignored, отдельный p10 gate exit 0;
workspace clippy --all-targets --locked -- -D warnings и fmt --check прошли.
Исходный Cargo.lock не менялся. Целевой build-каталог очищается штатным
cargo clean после сохранения evidence; новых бинарников для передачи не нужно.
