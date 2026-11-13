# SOCK_DIAG: фильтр портов без exact lookup

База: accepted origin/main 1db9ecf. PR142 уже включён в main; открытый PR148
85f163542cbc346917c20e5c0357904f1f48f88e не изменялся.

## Эквивалентность

Перед изменением прочитаны primary sources Linux:
- [v6.12 inet_diag.c](https://github.com/torvalds/linux/blob/v6.12/net/ipv4/inet_diag.c): inet_diag_dump_icsk, сравнения idiag_sport/dport, заполнение inet_diag_msg_common_fill.
- [v6.12 udp_diag.c](https://github.com/torvalds/linux/blob/v6.12/net/ipv4/udp_diag.c): udp_dump, те же сравнения для обоих семейств.
- [текущий tcp_diag.c](https://github.com/torvalds/linux/blob/master/net/ipv4/tcp_diag.c): tcp_diag_dump; обход established hash сохраняется.

Userspace требует точного равенства обоих портов. Ядро отсекает только записи
с отличающимся ненулевым портом запроса; нулевой порт оставляет wildcard.
Следовательно ни один возможный match не исключён. Bound-inactive TCP ветка
может игнорировать порты: это лишь избыточные записи, userspace их проверяет.
Адреса, интерфейс и cookie запроса не сужены. Несколько подходящих сокетов,
включая UDP reuseport, всё ещё дают AmbiguousSocket без cookie.
С cookie сохраняется сравнение полного tuple, а не поиск одного сокета ядром.

REQUEST|DUMP, all states, seq, kernel sender, DONE, truncation, overflow,
DUMP_INTR, record limit, cancellation и deadlines сохранены. Лимит записей
теперь применяется к выдаче с необходимым условием по портам; он не увеличен.
Нет новых unsafe, зависимостей, caps, pins или изменений ABI.

## Проверки

Сначала добавлены cfg(test) измерения send и receive до DONE, числа записей,
recvmsg и EAGAIN. Baseline: retained прошёл со второй попытки; candidate —
с первой. Это наблюдение на данном хосте, не гарантия времени планировщика.
Последний attribution run: 40 passed. Реальные TCP/UDP v4/v6, wrong cookie,
wrong tuple, equivalence с нефильтрованным dump; UDP reuseport ambiguity и
cookie selection проверены. Retained wire Known получил ttl_ns=11260328;
проверяется положительный TTL не более исходных 20_000_000 нс.
Существующие stale, executable replacement/deletion, epoch и revoke controls
сохранены и пройдены. Bootstrap по-прежнему предшествует наблюдению; snapshot
готовится после получения Lookup, lookup/project повторно выполняют witness.
TTL, timestamp, retry и deadline не ослаблялись.

clippy --lib --no-deps -D warnings: PASS. Полный all-targets clippy блокирует
существующий collapsible_match yonik-engine/src/lib.rs:170; scoped all-targets
блокирует существующий cloned_ref_to_slice_refs tests/resource_fd_receiver.rs:147.
Они вне scope. Отдельный opt-in cancellation gate на этом хосте не доходит до
capture: Broker::new возвращает Authority. Обычный attribution run этот gate
не активирует; успех реальной cancellation-проверки не заявляется.

Локальные полные логи, тестовый бинарник и SHA256 сохранены вне TTL:
/home/myrs/projects/yonik/local/evidence/diag-ports-20261112.
Сборки выполнялись с jobs=1 через with-rust-slot и flock собственного task root.
Полный Linux suite и воспроизведение GitHub runner не выполнялись.
