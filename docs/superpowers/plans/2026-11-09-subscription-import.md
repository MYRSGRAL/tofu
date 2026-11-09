# Начальный импорт подписок

Основание: FR-06/07, data-and-contracts.md §2.

1. yonik-import: ограниченное чтение, Plain/один Base64 envelope, максимум
   8 MiB входа, 32 MiB decoded, 20 000 непустых записей, 64 KiB на URI,
   глубина JSON 64. JSON проверяется без исполнения/загрузки файлов.
2. Нормализовать endpoint/port и credentials VLESS, VMess, Trojan, Shadowsocks,
   HTTP CONNECT, SOCKS5. Сохранить исходник и параметры. Неизвестные/опасные
   параметры помечать для review, не объявлять профиль готовым к активации.
3. Native JSON пока сохраняется opaque после синтаксической проверки; извлечение
   managed outbounds и согласование TUN/DNS/routing остаются следующим этапом.
4. Диагностический CLI yonik-import читает stdin и выводит только сводку и
   статические коды ошибок/индексы. Debug секретных типов скрывает содержимое.
5. Тесты malformed/partial/Base64/лимитов/редакции; приватную подписку проверить
   только в памяти. fmt/clippy/MSRV/release/Android, README RU/EN, review и PR.

Стандартные зависимости: base64, url, percent-encoding, serde_json/raw_value.
Никакого сетевого FetchHost, сохранения на диск, snapshot replacement или запуска
ядра в этом crate. Подтверждение частичного сохранения относится к будущему host.
