# Отмена waiters общего DNS exchange

R2: последний отменённый caller оставлял detached upstream до ответа или 8s;
синхронный retry также терял отмену survivor.

Xray CacheController теперь учитывает waiters каждого ключа. Отмена одного
caller сохраняет работу соседей; уход последнего удаляет запись и отменяет
upstream context. Завершение старого exchange не удаляет новую запись того же
ключа. Retry связывает собственный timeout context с отменой caller.
Новые публичные hooks, pins, ABI, capabilities и Rust не менялись.

Детерминированные тесты удерживают ответ без release и проверяют отмену upstream
и возврат caller за 1s, без ожидания штатного 8s timeout. На b09 оба теста RED;
на изменённом коде GREEN под race, включая изоляцию двух waiters. Go vet app/dns
успешен. Evidence: /home/myrs/projects/yonik/local/evidence/pipeline-e5a27,
файлы dns-red.log, dns-green.log, dns-vet.log. Это focused lifecycle evidence;
полная TUN приёмка оформляется отдельно.
