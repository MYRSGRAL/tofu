# Исходники и уведомления

Этот каталог содержит первичные LICENSE/NOTICE без изменения байтов и inventory
сохранённых Rust и Go компонентов. Rust inventory описывает normal/build closure,
а не доказанный состав каждого ELF. Go inventory описывает исследованные ядра
и может быть шире фактического module closure конкретной сборки. Controller-пакет
содержит Xray и sing-box, upstream/patched source archives, adapter patches
и Go source ZIP фактически включённых зависимостей. В legacy-варианте с внешним
поставщиком ядра устанавливаются отдельно; их состав и source obligations
проверяются у этого поставщика. Источник истины для конкретной поставки —
actual package entry list и связанный controller manifest, а не весь inventory.
Внешние Qt/OS пакеты устанавливает пакетный менеджер ОС, их 59-package closure
не объявляется содержимым YonIk.

Собственные исходники: https://github.com/MYRSGRAL/yonik . Точная revision исходного
архива controller-пакета закреплена collected record; сам архив поставляется как
usr/share/yonik/sources/release-source.tar. В legacy Arch recipe revision задаётся
PKGBUILD. Инструкции сборки находятся в packaging/arch/README.md.
Этот URL не является отдельным письменным предложением исходников и не заменяет
поставку соответствующих исходников там, где она требуется первичной лицензией.
До release необходимо сопоставить финальные ELF, включённые компоненты, первичные
лицензии и применимые обязанности; сохранить необходимые copyright/NOTICE,
исходники, patches и инструкции сборки. Выбор ветвей OR/WITH здесь не производится.

Release-source closure встроенного Rust/native/Qt header/generated кода требует
сопоставления с конкретными ELF и build inputs; наличие source archives и текстов
само по себе не подтверждает полноту. Исторические исследования source→ELF Xray,
Qt archive checksum и build cache сохранены в composition/P25-SCOPE.md в source
tree. Этот исторический файл поставляется под именем SCOPE.md в notices-каталоге
пакета; он описывает package78/source eec7, а не текущий controller payload.
Финальный current-main package требует отдельной чистой сборки после functional
merge. Этот набор ресурсов не подтверждает полную приёмку P25 или юридическую
готовность поставки.
