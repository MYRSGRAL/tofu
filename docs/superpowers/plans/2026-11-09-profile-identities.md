# Идентичность серверов и избранное

Основание: `data-and-contracts.md` §1 (display name не идентификатор), удобства
поиска/сортировки/избранного из requirements §3 и транзакционный контракт Repository.

1. Importer выдаёт версионированный канонический материал соединения и отдельное
   безопасное отображаемое имя. URI fragment и VMess `ps` не участвуют в identity;
   endpoint, credentials, transport/security и неизвестные опции участвуют.
   Материал чувствительный, Debug маскирован. Эквивалентный порядок query/JSON
   не меняет identity; разные явные defaults могут различаться консервативно.
2. Linux Repository выводит source-scoped HMAC fingerprint из master key,
   database ID и канонического материала. Обычный hash credentials не сохраняется.
   Schema v2 добавляет каталог в транзакции; v1 snapshots индексируются лениво.
3. Обновление snapshot и reconcile каталога выполняются в одном CAS commit.
   Дубликаты одного identity объединяются, первый parsed record задаёт имя/ordinal.
   Переименование и перестановка сохраняют ID/избранное. Пропавшее избранное
   остаётся недоступной записью и восстанавливается при возвращении identity;
   прочие пропавшие записи удаляются, числовые ID не переиспользуются.
4. Поиск/сортировка и страницы не требуют раскрытия body; metadata и избранное
   зашифрованы. Выбор ID перед активацией сверяет актуальный snapshot revision,
   присутствие и identity записи: старый ordinal не выбирает соседний сервер.

Проверки: шесть протоколов, переименование/параметры, Unicode и неизвестные
параметры; duplicates/reorder/remove/return; favorite против refresh и CAS;
миграция v1/будущая схема, отсутствие plaintext и tamper AAD. Затем MSRV,
workspace fmt/clippy/tests/release/Android и независимый review перед PR.

Это переносимая идентичность и backend списка для будущего GUI. Планировщик,
native JSON activation и отображение страниц в GUI этим этапом не завершаются.
