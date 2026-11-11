# P04 / FR-07: ограниченная проверка входа

База: `0f5e5333c1acc23aaedc4cd68724498beae483c1`.
Seed: `0x5040465230372026`; PRNG xorshift64 (13, 7, 17), 512 итераций на fixture.
Среда: rustc 1.97.0, cargo 1.97.0, Linux; jobs=1.

## Coverage → критерии

| Критерий | Уже существовавшее доказательство | Новое доказательство |
| --- | --- | --- |
| Размер входа/записи/числа записей, глубина | matrix.rs, invalid.rs | Не заявляется новым результатом |
| IDNA/IP, padding, percent, один слой Base64 | matrix.rs, variants.rs | 108 генерируемых комбинаций Trojan: 3 host × 3 port × 4 password × 3 представления; точные credentials, endpoint и ручной round-trip |
| Неоднозначные endpoints/users Xray | xray.rs, xray_protocols.rs | Не заявляется новым результатом |
| Неизвестные опции и original/native | matrix.rs, xray.rs | Fixtures обоих cores напрямую/в двух обёртках, побайтовое native/original; Xray future блокирует managed intent, sing-box остаётся opaque |
| Duplicate keys | xray.rs, xray_protocols.rs | Escaped nested duplicate в неизвестной опции обоих native fixtures, прямо/Base64 |
| Malformed legacy SS endpoint | Не покрыт скрытый fragment внутри Base64 | RED → одна проверка fragment → GREEN; query/path/пустой fragment, соседняя валидная запись; позитивный контроль пароля и внешнего display fragment |
| Mutation smoke | Отдельного corpus runner нет | 1536 мутаций × 3 представления × 2 повторных разбора = 9216 вызовов original import_bytes API |

Мутации: замена байта, удаление, вставка из фиксированного алфавита и усечение.
Максимальный mutated input — длина fixture + 1 до обёртки. На каждом вызове
проверяется сохранность исходных байтов при успехе/ошибке; два вызова должны
совпасть по категории отказа либо отчёту, native, source, parameters, identity,
endpoint и review flags. Panic проваливает тест. Нет wall-clock assertions.

## Проверки

- `red.log`: регрессия падает на `#hidden` (принято 2 сервера вместо 1).
- `green.log`: `cargo test -p yonik-import`, 77 passed, включая 4 новых теста и doctest.
- `clippy.log`: `cargo clippy -p yonik-import --all-targets -- -D warnings`.
- `rustfmt --edition 2024 --check` для двух изменённых Rust-файлов; `git diff --check`.

Все тяжёлые cargo запускаются через with-rust-slot с CARGO_BUILD_JOBS=1,
CARGO_TARGET_DIR в собственном transient и flock самого task-root.
Логи здесь вне TTL; исполняемые файлы для результата не требуются.

## Ограничения и оставшиеся gates

Это bounded deterministic property/mutation smoke, не fuzz campaign.
Отсутствие panic/hang/аномальных аллокаций наблюдалось только на этих входах;
универсальная гарантия памяти/времени не доказана. Отдельных fuzz targets и
готовой fuzz-инфраструктуры в репозитории не найдено; dependencies не добавлялись.
Остаются coverage-guided fuzz-duration gates с согласованным бюджетом времени,
coverage, sanitizers/allocator instrumentation, расширением corpus и отдельными
стресс-измерениями памяти/времени на лимитах. Spec не задаёт численного fuzz-duration
порога; этот PR такой gate не закрывает. MAX_DECODED_BYTES не достигается обычным
Base64 при меньшем MAX_INPUT_BYTES; самостоятельное доказательство этого лимита
не заявляется. Ручной контракт общий для cores; engine config-check не запускался.
Sing-box managed extraction отсутствует в этом API, проверена только сохранность
native. Старые matrix45/VMess-Trojan51/ALPN/DNS72 не считаются новым покрытием.

## Полнота алфавита вставок после review

Root regression с проверкой фактически использованного набора воспроизвела
3/12 символов. Выбор символа теперь использует счётчик вставок (`iteration / 4`),
независимый от номера операции. Сама mutation-проверка требует появления всего
алфавита; seed, 1536 мутаций и 9216 parser calls не изменились.

После исправления общий suite дал 77 passed. All-targets Clippy выявил в этом
же тесте `format_collect`: percent-encoding теперь пишется в один String с
заданной capacity. После этой последней правки прошли четыре targeted tests,
включая mutation и 108 roundtrips, и строгий all-targets Clippy через явно
закреплённый driver 1.85.1. Это не расширяет доказательства на полноценный fuzz
campaign или предельные memory/time budgets.

Root evidence: `local/validation/pr81-mutation-alphabet/` основного workspace:
`red.log`, `green.log`, `clippy-pinned-format-collect.log`, `final.log`.
Оригинальный авторский `green.log` сохранён там как `author-green-original.log`;
в repo-копии удалена только лишняя завершающая пустая строка ради diff-check.
