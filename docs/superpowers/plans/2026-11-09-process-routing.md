# Нативное сопоставление executable в Linux TUN

Основание: FR-20 и G-APP, docs/specs/routing-and-tray.md.

1. Новый user/network namespace и localhost echo из проверенного traffic-стенда.
2. Две отдельные копии Python executable в разных каталогах с одинаковым basename.
   Каждый тестовый клиент подтверждает фактический `/proc/self/exe`.
3. Для выбранного точного пути компилировать синтетическое process + CIDR правило
   ядра; совпавшее правило перенаправляет к echo, остальные потоки блокируются.
4. Запустить TCP/UDP IPv4/IPv6 обоих клиентов. Поменять разрешённый executable
   и повторить; источник должен менять результат при неизменном назначении.
5. Проверить конфиги, TUN counters, cleanup и отсутствие лишнего engine,
   оформить evidence, выполнить Ruff/Python/Rust проверки и независимый review.

Файлы: tools/engine_apps.py, tools/app_probe.py. Используются синтетические
конфиги, никаких пользовательских подписок. Все временные копии/конфиги находятся
в task-каталоге под flock корня и удаляются после проверки.

Это evidence точного пути в ограниченном стенде с одинаковым UID. Не доказаны
app catalog, дочерние процессы браузера, sandbox/Flatpak/Wine, чужие UID,
systemd hardening, proxy outbound, DNS attribution и стоимость распознавания.
Полный G-APP по одному baseline не закрывается.
