# Дорожная карта

Состояние на 24.11.2026. Внутри разделов порядок — сверху вниз.

## Сделано

- [x] Перенос базы Asterinas 0.19.0, новая история коммитов
- [x] Форматирование по rustfmt с настройками базы
- [x] README, логотип, лёгкий CI
- [x] Реализован syscall `membarrier`: QUERY, GLOBAL, EXPEDITED с регистрациями через IPI-барьеры OSTD
- [x] Пользовательские строки переименованы: логи загрузчика, меню GRUB, сообщения сборки
- [x] Сборка ядра и ISO в чистом окружении контейнера зафиксирована в CI
- [x] Тесты ядра (ktest) гоняются в CI на каждый push в main
- [x] Workflow Release: ISO и ELF собираются по тегу `v*`, публикуются в GitHub Releases
- [x] Раздел в `book/` про отличия Aporia от upstream, включая политику синхронизации
- [x] Release pipeline проверена тегом `v0.19.0-aporia.1`: ISO и ELF опубликованы в GitHub Releases
- [x] Тяжёлый тестовый контур: workflow `Test` по недельному расписанию и вручную — OSDK-тесты в двух контейнерах, boot и regression, conformance (ltp, gvisor, kselftest), xfstests по short.list (ext2, overlay, tmpfs, virtiofs), pjdfstest, кросс-компиляция boot-теста на ARM
- [x] Полный xfstests по full.list выделен в отдельный workflow и честно пропускается на машинах без `/dev/kvm`
- [x] Кэши сборки OSDK и ядра шарятся между job'ами тяжёлого контура
- [x] Быстрый workflow `Test smoke` для проверки контура вручную: boot, regression, xfstests, pjdfstest
- [x] Инструкция по загрузке на реальном железе и честный список драйверной поддержки в `book/`
- [x] Тяжёлый контур прогнан до конца: 32 из 33 job'ов зелёные; упавший lint (fmt-дрейф в примерах книги) разобран и починен
- [x] Восстановлены док-комментарии из базы после чистки: `make check` (fmt, clippy, udeps) проходит целиком
- [x] Партия syscalls: `copy_file_range`, `recvmmsg`, `clock_getres`, `times`, `get_robust_list`, `rt_sigqueueinfo`, `rt_tgsigqueueinfo` с регрессионными тестами в initramfs
- [x] Семейство mlock (mlock, mlock2, munlock, mlockall, munlockall): populate страниц через синтез page fault, флаг VM_LOCKED на маппингах, учёт в RLIMIT_MEMLOCK, MCL_FUTURE/MCL_ONFAULT, регрессионный тест memory/mlock
- [x] Партия syscalls: `process_vm_readv`/`process_vm_writev` (доступ к чужой памяти через read_alien/write_alien + проверка ptrace-пермишеней) и `splice`/`vmsplice`/`tee` (tee — через peek ring buffer без потребления); регрессионные тесты ipc/pipe/splice и process/process_vm

## Ближайшие шаги

- [ ] Первая загрузка на реальной машине и первая запись в список проверенного оборудования

## Дальше

- [ ] Расширять покрытие Linux syscalls по списку отсутствующих (124 номера, крупные кандидаты: splice/vmsplice/tee, POSIX-очереди сообщений, SysV IPC, rseq, process_vm_readv/writev, openat2, statmount)
- [ ] Драйверы для реального железа: AHCI/SATA, USB-хост, сетевые карты
- [ ] Свой инсталлятор и дистро-слой поверх ядра
