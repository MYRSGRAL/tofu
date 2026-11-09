# Контур подключения и остановки

Основание: architecture.md §5–7, data-and-contracts.md §5.

1. Создать yonik-application без ОС/GUI/runtime и сторонних зависимостей.
2. Проверить тестами последовательность Preparing → Starting → Checking →
   Connected/Limited, запрет Connected по одному запуску, busy и неверные события.
3. Каждое принятое событие получает новый opaque token; stale callbacks
   игнорируются даже при повторном входе в то же состояние. Stop отменяет старый token.
4. Ошибка после начала запуска требует cleanup. Только CleanupConfirmed позволяет
   Disconnected/Error; CleanupFailed остаётся отдельным состоянием и требует retry Stop.
5. Проверить fmt/clippy/tests/release и Android compile-check общей библиотеки,
   обновить README RU/EN/evidence, провести review и интегрировать атомарные коммиты.

Это первый чистый reducer, а не готовый application host. Профили, реальный Apply,
журнал сетевых ресурсов, persistence, retry/recovery и reconfiguration пока не
реализуются. Host подтверждает события; типы reducer не доказывают сетевой результат.
