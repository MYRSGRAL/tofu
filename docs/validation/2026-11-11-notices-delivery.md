# Доставка notices пользователю

База: 9ec9da07a3ee64bc9d2a2d2d72c4d97a54bcfe14. Android deferred.

About включает поиск и виртуализированный список 225 Rust packages и 173 Go
modules из сохранённых inventory. Go явно обозначены как исследованные внешние
ядра, отсутствующие в planned Arch payload. Диагностика пользователя не читается.
Компилятор ресурсов использует root-bound O_NOFOLLOW reader, сверяет SHA-256,
UTF-8, непустоту и пределы: 512 записей, 4 MiB на файл, 16 MiB уникальных текстов.
В qrc попадают индекс и отдельные QML-объекты текстов. UI создаёт только выбранные
объекты; локальный XHR и network не нужны. Оригиналы сохраняются без нормализации.

PKGBUILD устанавливает оригинальные тексты, index, оба inventory, LICENSE,
SCOPE и SOURCE-OBLIGATIONS в /usr/share/licenses/yonik. Сгенерированные QML и qrc
не устанавливаются и не коммитятся. Archive validator проверяет точные bytes и
права, лимит числа entries расширен на ограниченный expected inventory.

Проверки: Python — 10 tests и 34 subtests PASS; Ruff check/format PASS;
Rust 1.85.1 rustfmt PASS; QtTest 6.11.2 offscreen — 3 PASS, включая 398 записей,
поиск, ресурсный текст, отказ произвольным путям, обе локали и PlainText.
QtTest использовал системный Qt в отдельном offscreen процессе, не отдельную
private Qt installation. Общий translation checker проверяет Main.qml;
NoticesView имеет явные пары RU/EN для отдельного ALLQML review.

Рецепт по-прежнему закреплён на историческом eec7 source archive и его checksum.
Это planned изменение: новый исходный архив/revision/checksum и current-main ELF
после functional merge ещё нужны. Новый seal отражает изменение рецепта и требует
независимого review перед принятием. Пакет не выпускался, host install не выполнялся.
Не подтверждены Xray source→ELF, три Qt checksum и build-cache provenance;
первичные source obligations окончательной поставки требуют отдельной оценки.
Полный P25 PASS и юридическая готовность не заявляются.

Генератор ресурсов требует python3 в build environment. Зависимости по ограничению
задачи не менялись: добавление python в makedepends перед clean Arch build требует
координации с root. Без этого чистая среда, не имеющая Python, не поддерживается.

Повторить QtTest в собственном transient root под flock:
`python tools/compile_notices.py "$task/resources"`,
`rcc --binary "$task/resources/notices.qrc" -o "$task/notices.rcc"`,
`c++ tools/tests/notices_qt.cpp -o "$task/test" $(pkg-config --cflags --libs Qt6QuickTest)`,
`YONIK_NOTICES_TEST_RCC="$task/notices.rcc" QT_QPA_PLATFORM=offscreen "$task/test" -input tests/qml/notices`.
Сборочный каталог после проверки очистить, лог сохранить вне TTL.

Pinned Rust 1.85.1 Clippy `--offline --locked -p yonik-desktop --all-targets --features gui -- -D warnings`: PASS, 5m08s. Использован общий two-slot wrapper, root flock, jobs=1 и собственный внешний target. C++/Qt выдал предупреждения SFINAE, проверка завершилась exit 0. Manifest был записан после старта; pre-build provenance не заявляется.
