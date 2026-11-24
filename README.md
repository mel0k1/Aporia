<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
    <img src="assets/logo-light.png" alt="Aporia" width="160">
  </picture>
</p>

<h1 align="center">Aporia</h1>

<p align="center">
  Операционная система на базе ядра Asterinas<br>
  <a href="https://github.com/mel0k1/Aporia/actions/workflows/ci.yml"><img src="https://github.com/mel0k1/Aporia/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE-MPL"><img src="https://img.shields.io/badge/license-MPL--2.0-blue" alt="License: MPL-2.0"></a>
</p>

Aporia строится на ядре [Asterinas](https://github.com/asterinas/asterinas) 0.19.0. Ядро написано на Rust: небезопасный код собран в компактном фреймворке OSTD, остальное написано в safe Rust. Такой подход называется framekernel: производительность монолитного ядра и изоляция модулей на уровне языка.

База перенесена целиком, история коммитов начата заново.

Имя — отсылка к апории: рассуждение упирается в противоречие, и его нужно разбирать, а не обходить.

## Что уже умеет база

- x86-64 (Tier 1), ARM64, RISC-V 64, LoongArch 64 (Tier 2–3)
- Совместимость с Linux: 240+ системных вызовов
- Графика через DRM, virtio, NVMe, сеть, FUSE, шифрованный многослойный диск mlsdisk
- Конфиденциальные вычисления: Intel TDX
- OSDK — тулкит для сборки, запуска и тестирования ядра

## Сборка и запуск

Нужен Linux x86-64 (или ARM64) и Docker.

```bash
git clone https://github.com/mel0k1/Aporia
cd Aporia
docker run -it --privileged --network=host -v /dev:/dev -v $(pwd):/root/asterinas asterinas/dev:0.19.0-20261012
make kernel
make run_kernel
```

Внутри контейнера поднимается виртуальная машина с ядром Aporia и минимальным initramfs. Полноценный дистрибутив собирается командами `make nixos` и `make run_nixos`.

## Структура

| Каталог | Содержимое |
| --- | --- |
| `kernel/` | Ядро: системные вызовы, подсистемы, компоненты |
| `ostd/` | Небезопасный базовый фреймворк |
| `osdk/` | Инструменты разработки |
| `book/` | Документация (mdBook) |
| `test/` | Интеграционные тесты |

## Планы

Первый этап закрыл пробелы уровня репозитория: брендинг, CI, проверки кода. Дальнейший порядок работ — в [ROADMAP.md](ROADMAP.md).

## Происхождение и лицензия

База — [Asterinas](https://github.com/asterinas/asterinas), лицензия MPL-2.0 сохранена без изменений: файлы [LICENSE-MPL](LICENSE-MPL) и [COPYRIGHT](COPYRIGHT). Отличия от upstream видны в истории коммитов.
