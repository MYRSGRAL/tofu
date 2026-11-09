# Фактические данные Linux NetworkHost

Следующий этап после наблюдения поколений — read-only данные ядра. Статический
`LOCAL_ENV` не превращается в системный lease; `PrivateNetworkBinding` остаётся
неподдержанным до полной проверки DNS, прав, владения и Apply.

1. Общий безопасный RTNETLINK reader: только GET-операции, kernel sender, seq,
   multipart/DONE/error, DUMP_INTR, truncation/ENOBUFS, общие лимиты и отмена.
   Подписка multicast предшествует чтению; события во время сбора отвергают попытку.
2. Typed topology: links, IPv4/IPv6 addresses, маршруты всех таблиц, policy rules,
   nexthop objects/groups. Неизвестные атрибуты сохраняются как явная неполнота;
   снимок не считается разрешением менять сеть.
3. Отдельный kernel route-get с фактическими UID, mark, source и параметрами
   транспорта. Выбор Linux policy routing не воспроизводится своим алгоритмом.
4. Unit/protocol tests и native namespace gate: сверка таблиц/селекторов,
   изменения во время dump, ошибки и лимиты, отсутствие мутаций host namespace.
   MSRV/Clippy/Android, README RU/EN, независимое ревью и PR после проверок.

Снимок означает завершённое ограниченное чтение без замеченных изменений, а не
атомарный snapshot ОС. DNS/ResolverHost, ApplicationCatalog, authorization lease,
helper journal и Validate/Apply fence остаются отдельными необходимыми частями.
