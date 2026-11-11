# System controller: ordinary merge с принятым main

Исходный review head: `53e9e1ede097249bf60f3ab7183abe8ed1c738f5`; merge head: `5ee83738cbfa470a98734c86636e648e07c65d13`.
Обычный merge с main `d0a281b4c23b45f65db1ddb74c6b14378b399794` выполнен без конфликтов.

Перед публикацией побайтово сверены backend.rs, system_session.rs, main.rs,
application.cpp и application.h с reviewed head53 — идентичны. Sticky recovery
guard и его поведение не переписаны при merge. Принятые изменения main (DNS,
presets, import corpus и RU/EN catalog) сохранены.

Проверки merge: cargo fmt --check, check_translations.py, git diff --check.
Прежние RED/GREEN unit/Qt proof относятся к head53; повторного runtime/build
на merge head не было. Новые CI должны пройти на точном опубликованном head.
Production GUI/TUN/transport и общий P19 этой интеграцией не принимаются.
