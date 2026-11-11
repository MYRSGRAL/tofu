
## Recheck границы scoped gate

Root заменил общий `RowLayout {` на уникальную границу `routingDefaultRow`.
Проверяется единственность границы и наличие всех пяти обязательных objectName
внутри извлечённого Routing-участка. Добавление вложенного RowLayout больше не
скрывает Explain, а исчезновение обязательного элемента вызывает отказ.
Два mutation RED → GREEN, 17 Python tests, Qt 3 test results и checker/Ruff PASS.
Общий pair-validator остаётся более узким; CLI обязательно выполняет scoped gate.
Лексический gate не объявляется полноценным QML parser.

Новый delta-manifest и логи:
`/home/myrs/projects/yonik/local/validation/pr117-scope/`.
