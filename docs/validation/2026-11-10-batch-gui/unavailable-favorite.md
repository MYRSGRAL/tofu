# PR76 P2: исчезнувший favorite не блокирует TCP-пакет

Review base: `d9351bcffd338cf4fbdd3e9b95305afba4e68067`.
Merge с main сохранён; rebase/reset/amend не выполнялись.

## RED → GREEN

На неизменённом production-коде d935 три новые регрессии завершились FAIL:

- A+B → favorite A → refresh only B: batch возвращал NotFound вместо B.
- 10 000 available + один historical favorite: InvalidInput из-за count(*).
- Ранний NotFound на historical строке скрывал Corrupt текущей строки B.

После исправления все 7 repository batch tests прошли. Точный DesktopHost bridge
через GUI JSON protocol также прошёл: импорт A+B, favorite A, refresh only B,
check_batch даёт total=completed=1 и ID/revision B; запрос favorites по-прежнему
возвращает A с favorite=true, available=false. Ошибки refresh_required нет.

Лимит и страницы проверены отдельно: 10 000 available + historical A допустимы,
страницы available с offset 0/9999/10000 имеют total=10000 и длину 1/1/0;
10 001 available отклоняются. Исторический favorite остаётся в БД и favorite query.

## Изменение и целостность

Production diff только в repository/batch_targets.rs. Сначала read_profiles
читает bounded каталог, open_profile проверяет AEAD каждой строки, в том числе
historical. Только после успешной аутентификации available используется для
отбора. SQL present не фильтруется до AEAD: этот флаг входит в AAD профиля.

Historical строки не сверяются с текущей revision/fingerprint текущего snapshot:
это штатные сохранённые favorites прошлого snapshot. Available строки сохраняют
проверки snapshot generation/revision, ordinal и fingerprint. Число
аутентифицированных available сверяется с catalog unique_count и только затем
ограничивается 10 000. Разбор snapshot остаётся однократным, без секретных
ParsedServer в выходе. Алгоритм линейный по сохранённым строкам и snapshot;
существующий read_profiles limit 40 000 не менялся.

Сохранены current source generation CAS, expected revision, catalog generation,
raw/unique bounds, snapshot/profile AEAD, cancellation и producer ACK/fences.
Повреждения historical payload, historical present, скрытие current B через
present и подмена current revision возвращают Corrupt, не пропускаются фильтром.
Backend/system GUI/общие repository helpers, схема, подписи API не менялись.

## Релевантная проверка после фикса

- Repository bulk: 7 PASS, включая прежние CAS/equivalence/AEAD/queued cancel.
- Desktop unit: 22 PASS; backend bridge: 35 PASS, 1 чужой native gate ignored.
- ActualHost 10k: 10 000 уникальных IDs/results, ровно 30 000 TCP EOF на 16
  собственных loopback listeners (1 875 каждый), все страницы проверены.
  Серия и чтение результатов: 6 200 мс debug.
- CPU prepare Cancel/Stop/Shutdown: 154/155/156 мс, EOF после ACK не растут.
- RTNETLINK epoch 1→2→3: completed/in-flight stale очищаются; отмена 8 мс.
  Собственный user/net namespace, host routes не менялись.
- Clippy linux+desktop --all-targets --no-deps -- -D warnings: PASS; fmt выполнен.

Чужие VM/native gates и GUI build не повторялись. Root сообщает новый head
SystemGUI writer для обычного merge и узкого review исправленного P2.

## Evidence

`local/evidence/batch-unavailable/`: red.log, green.log, backend.log, gates.log,
clippy.log, provenance.json, cleanup.log. Бинарники сохранены вне build TTL;
точные SHA256 и source blobs связаны с новым commit в provenance. Старый merge
base и все исходные результаты не переписываются.
