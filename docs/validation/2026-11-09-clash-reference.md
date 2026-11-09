# Разбор Clash Verge Rev как референса

Изучен [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev/tree/9d0bee4af9a81565ca24637bebe4485ac485a690)
на commit `9d0bee4af9a81565ca24637bebe4485ac485a690`: структура проекта,
архитектура React/Tauri, целевые пути запуска, подписок, настроек, списков,
службы и Linux-интеграции. Это обзор архитектуры и соответствующих исходников,
не построчный аудит всего репозитория и не измерение его производительности.
Временный read-only clone удалён; зависимости и сборки не запускались.

Полезные паттерны для tofu:

1. [Single instance](https://github.com/clash-verge-rev/clash-verge-rev/blob/9d0bee4af9a81565ca24637bebe4485ac485a690/src-tauri/src/utils/server.rs):
   ранняя блокировка и передача show/deep link. В tofu уже есть user-only
   QLocalServer; HTTP/token не нужны. Повторный запуск может показать старый
   экземпляр, поэтому добавлен видимый build ID. Гонки старта требуют тестов.
2. [Порядок старта](https://github.com/clash-verge-rev/clash-verge-rev/blob/9d0bee4af9a81565ca24637bebe4485ac485a690/src-tauri/src/utils/resolve/mod.rs):
   критичные миграции до окна, тяжёлые эффекты отдельно. В tofu изменения
   startup следует обосновывать cold-start измерением, а не переносить всё в фон.
3. [Применение настроек](https://github.com/clash-verge-rev/clash-verge-rev/blob/9d0bee4af9a81565ca24637bebe4485ac485a690/src-tauri/src/feat/config.rs):
   сериализация patch, разделение effect/persistence и видимая ошибка сохранения.
   Для HWID tofu использует существующие generation/CAS и атомарную запись.
4. [Виртуализация](https://github.com/clash-verge-rev/clash-verge-rev/blob/9d0bee4af9a81565ca24637bebe4485ac485a690/src/components/base/virtual-list.tsx):
   создавать видимые строки с ограниченным запасом, ограничивать частоту
   обновлений. В tofu сохраняются QML ListView и пагинация; для будущей
   оптимизации измерять число делегатов/model reset, а не менять toolkit вслепую.
5. [Параметры подписки](https://github.com/clash-verge-rev/clash-verge-rev/blob/9d0bee4af9a81565ca24637bebe4485ac485a690/src-tauri/src/config/prfitem.rs):
   per-source UA/timeout и metadata подписки. HWID в просмотренных src/src-tauri
   не найден; его контракт для tofu взят непосредственно из Remnawave.
6. [Состояния службы](https://github.com/clash-verge-rev/clash-verge-rev/blob/9d0bee4af9a81565ca24637bebe4485ac485a690/src-tauri/src/core/service.rs):
   различать checking/ready/not-installed/mismatch/unavailable. Для будущего
   helper tofu нужны такие состояния и capabilities, а не один флаг TUN.

Не переносить WebView/React, HTTP singleton, WebKit/NVIDIA-обходы в нативный Qt.
Отключение проверки TLS и скрытый direct fallback не соответствуют контракту tofu.
Ни один из этих исходников не доказывает меньший расход памяти или плавность tofu:
сравнительный benchmark остаётся отдельной проверкой.
