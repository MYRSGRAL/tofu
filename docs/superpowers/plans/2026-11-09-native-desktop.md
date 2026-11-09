# Первый нативный desktop-host

Основание: architecture §2–6, FR-04/05/06/10/19, контракт трея T-01…T-04.
Рабочий выбор Codex — Qt Quick/CXX-Qt 0.10.0 для проверки на KDE Wayland;
это не пользовательское утверждение визуального дизайна и не закрытие G-UI.

1. Проверить настоящий Cargo-only CXX-Qt build, кириллицу, ввод и Qt resource
   path на Rust 1.85.1/Qt 6.11.2. GUI optional feature оставляет domain/backend
   независимыми от Qt и доступными обычному CI/Android compile-check.
2. Один Qt UI-поток, отдельный Rust/Tokio host и ограниченные blocking-задачи.
   QML передаёт только allowlisted команды; backend публикует bounded DTO.
   Доставка в Qt coalesces последний snapshot без постоянного polling.
3. Рабочие экраны: состояние локального SOCKS, подписки, поиск/избранное/выбор,
   подтверждение частичного импорта, выбор ядра и RU/EN. Системный TUN/proxy,
   Fake DNS и редактор маршрутизации не изображаются работающими переключателями.
4. Контроллер использует существующие проверенные snapshot/engine adapters;
   Stop/Shutdown имеют отдельный приоритетный путь. Активный immutable server
   отделён от выбранного. Нормальный выход ждёт stop/reap и закрытие Repository.
5. QApplication/QSystemTrayIcon: скрытие только при доступном трее, восстановление,
   команды через тот же controller, один экземпляр на data directory. Без трея
   окно не исчезает без способа управления. Ошибка cleanup не выдаётся за выход.

Визуальное направление: спокойный утилитарный интерфейс, системный шрифт,
нейтральный тёплый фон и сдержанный зелёный акцент состояния. Основное действие —
подключить выбранный сервер; статус Limited явно отделён от проверенного HTTPS.
Названия и адреса рендерятся plain text. Секретный URL вводится скрыто, очищается
после передачи, не входит в DTO, трей, argv и diagnostic export.

Общее workspace `unsafe_code=forbid` не меняется. В новом GUI crate backend lib
имеет `forbid`, handwritten binary impl — `forbid/deny`; только declarative
generated CXX bridge module получает точечное `allow`, необходимое для FFI.
C++ слой владеет Qt-объектами через RAII в UI-потоке; Rust worker касается Qt
только через CxxQtThread.queue и не удерживает сырой QObject после удаления.

Проверки: backend actor с ephemeral тестовым ключом и loopback HTTP; настоящая
GUI-сборка, QML lint, native smoke с собственной window capture и fake URI;
Stop во время операции, closed acknowledgment, запуск без трея/в трее и
повторный запуск. Приватные данные тестовой подписки не сохраняются открыто.
Затем fmt/clippy/tests/release/Android, проверка GUI feature, независимый review,
README RU/EN и PR. При недоступном GitHub CI использовать ранее разрешённый
локальный fallback только после сверки exact-head annotations.
