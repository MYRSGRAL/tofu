# Offline capture: два P2 из review e2794230

Исправление не выполняет extraction/boot и не изменяет historical receipts.
Все три offline subprocess пути используют общий capture с суммарным raw cap
stdout+stderr 1 MiB, абсолютным timeout и bounded kill/reap. На overflow/timeout
нет анализа частичного output как graph PASS; сессия завершается с failure.

Каждый subprocess создаётся в отдельной owned session. Leader наблюдается через
waitid(WNOWAIT), PID/PGID не освобождается до обязательного killpg, включая
нормальный выход; затем wait до1s и закрытие pipes. Отдельный capture_failure
фиксирует timeout/overflow/reap_timeout. Последний означает UNKNOWN, не cleanup PASS.

Directory locks shared от первого использования; unit-selector после подтверждения
существующей session получает собственный shared lock. Root/base/input lock FDs
передаются tool через pass_fds: при неподтверждённом reap живой child сохраняет
custody соответствующих каталогов. Cleanup оставляет task при найденных references.
Как и раньше, внешние tools доверены; descendant, сознательно закрывающий унаследованные
FD или escaping session, этим контрактом не добавлен в поддерживаемую модель.

Тесты используют только собственные Python children и temporary directory.
Baseline61f: 2 MiB output принимался без отказа — assertion RED. Новый код: cap,
stdout+stderr together, normal reap, timeout, удерживаемый parent writer и
унаследованный lock при mock unconfirmed reap — 6 PASS; offline suite20 PASS.
Ruff format/check и diff-check PASS. Хеши в соседнем JSON; logs внеTTL.
Ни btrfs/journalctl, ни VM, mount/repair/SSH/product activation не запускались.
Пять reviewed serial source hashes не менялись; будущий serial boot требует
точного root decision и не разрешается этим коммитом. FullA4/finalLinux=false.
