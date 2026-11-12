# Typed probe context: известная причина до legacy mapping

Production session сохраняет HTTP-status failure, Timeout, TLS и Transport в
закрытом Stage::Probe. HostError во время check/invalidate даёт общий
ProbeCheckFailed; ожидаемая Cancelled не публикуется как ошибка. Потеря сети
публикует ProbeNetworkChanged. Прежние code/message-key, Connected/Limited,
DTO результата, stop/cleanup и V1 не изменены. URL/status/OS error не добавляются
в params: они строго пусты. Низкоуровневая причина из Transport не выдумывается.

Production builder SessionUpdate вынесен без изменения поведения для RED,
затем получает typed companion. Actor сохраняет existing generation и network
fences; contexts заменяются при current success и network invalidation.

Проверки Rust1.85.1 offline/locked: RED показывает отсутствие metadata у actual
producer. До clippy-only then→then_some упрощения: lib98 PASS/1ignored,
backend54 PASS/2ignored. После него: обе targeted producer/network tests PASS,
clippy test-support all-targets -D warnings и fmt PASS. Тесты проходят настоящий
builder→actor publication→strict V2, проверяют privacy, late generation и clear.
Python V2 5/5 и factory2/2 PASS. GUI/native campaign не повторялась.

Evidence: /home/myrs/projects/yonik/local/evidence/p11-probe-causes/.
red-backend.rs — ранний draft до исправления imports/формы assertions, не exact
compiled RED source. Подтверждение RED — terminal test log; ограничение exact
RED source binding сохраняется. source-before.json относится к broad GREEN до
clippy упрощения; final manifest отдельно фиксирует итоговые source hashes и
retained test binary. Старые прерванные/неуспешные логи сохранены.

Это закрывает probe producer boundary; полный P11 и native admission здесь
не принимаются. Никаких deps/pins/unsafe/HostConfig API изменений нет.
