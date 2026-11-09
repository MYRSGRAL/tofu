# Рабочий нативный прототип А

Пользователь выбрал А и просит доступный новый UI. Перенести его сейчас на
существующий Qt Quick backend. Выбор окончательного toolkit и энергоприёмка
остаются отдельным этапом и не задерживают визуальный прототип.

1. Main.qml: тёмная палитра highpriority.cloud, выбранный/активный сервер,
   центральная кнопка connect/stop/cancel, статус, gear. Без выдуманной страны,
   ping/скорости или успешного TUN. Данные plain text, все тексты RU/EN.
2. Отдельный picker с виртуализацией/страницами200/search/favorites; настройки
   включают ядро, локальный порт, источники/refresh review, TCP/HTTPS и язык.
   Сохранить работающие команды, секретные URL очищать, capability/error guards,
   tray/close lifecycle. Недоступные TUN/Fake-IP/kill switch не выдавать за applied.
3. Проверить реальные пользовательские действия и состояния на собственном Qt
   окне, empty/error/connected/limited/cleanup, клавиатуру/scaling и screenshots.
   Тесты backend, GUI smoke, review, CI; сохранить release и дать команду запуска.
