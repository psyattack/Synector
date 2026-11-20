<p align="center">
  <img src="assets/app.svg" alt="Логотип Synector" width="120" height="120">
</p>

<h1 align="center">Synector</h1>

<p align="center">Лёгкий нативный набор автоматизации Windows и супервизор процессов, написанный на 100% чистом Rust.</p>

<p align="center">
  <a href="https://github.com/psyattack/Synector/releases/latest"><img alt="Последний релиз" src="https://img.shields.io/github/v/release/psyattack/Synector"></a>
  <a href="https://github.com/psyattack/Synector/releases"><img alt="Загрузки" src="https://img.shields.io/github/downloads/psyattack/Synector/total"></a>
  <a href="https://github.com/psyattack/Synector/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/psyattack/Synector/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="Лицензия: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/psyattack/Synector/releases/latest"><img alt="Платформа" src="https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0078D6"></a>
</p>

<p align="center">
  <a href="README.md">English</a>
  ·
  <strong>Русский</strong>
</p>

<p align="center">
  <a href="https://psyattack.github.io/Synector/?lang=ru">Сайт</a>
  ·
  <a href="https://github.com/psyattack/Synector/releases/latest">Скачать</a>
  ·
  <a href="https://github.com/psyattack/Synector/issues/new">Issues</a>
</p>

> [!WARNING]
> Synector напрямую взаимодействует с системными процессами Windows и административными API. Разработчики не несут ответственности за любые ошибки, в том числе вызванные пользователем, как указано в лицензии.

<p align="center">
  <img src="assets/screenshots/rules.png" alt="Список правил Synector" width="49%">
  <img src="assets/screenshots/overlay-studio.png" alt="Overlay Studio в Synector" width="49%"><br>
  <img src="assets/screenshots/macro-studio.png" alt="Macro Studio в Synector" width="49%">
  <img src="assets/screenshots/analytics.png" alt="Аналитика Synector" width="49%">
</p>
<p align="center"><sub>Правила · Overlay Studio · Macro Studio · Аналитика</sub></p>

## Содержание

- [Обзор](#обзор)
- [Возможности](#возможности)
- [Установка](#установка)
- [Разработка](#разработка)
- [Конфигурация](#конфигурация)
- [Авторство правил из CLI](#авторство-правил-из-cli)
- [Документация](#документация)
- [Лицензия](#лицензия)
- [Благодарности](#благодарности)
- [Поддержка и участие](#поддержка-и-участие)

## Обзор

Synector поставляется одним исполняемым файлом Windows с двумя уровнями. Тихий **демон** в трее
круглосуточно владеет движком правил, триггерами, счётчиками времени выполнения и мониторингом
системы. Аппаратно-ускоренный **GUI** (eframe/egui) — управляющий клиент: он редактирует черновики
правил, показывает состояние демона и закрывается, не останавливая автоматизацию.

Synector общается с Windows напрямую: Win32 API, именованные каналы Win32, встроенный Lua — без
браузерного движка, встроенного веб-рантайма и телеметрии. Обнаружение событийное там, где Windows
предоставляет источник событий: ETW ядра для жизненного цикла процессов, низкоуровневые хуки при
записи макросов, HTTP-webhook, дельты камеры через Raw Input, широковещательные сообщения о питании
для сна и пробуждения, состояния дисплея и уведомления Plug and Play об устройствах. Фокус активного
окна, изменения файлов, пороги ресурсов, блокировка сеанса, питание от сети или батареи и состояние
сети опрашиваются небольшими адаптивными циклами — без попыток выдать опрос за событие.

## Возможности

* **Два уровня на чистом Rust** — демон в трее владеет правилами, триггерами и мониторингом;
  закройте GUI, и автоматизация продолжит работать почти без нагрузки на CPU.
* **Гибридное обнаружение событий** — трассировка процессов через ETW ядра при повышенных правах
  и адаптивный опрос снимков Toolhelp32 без них; оба пути питают одну и ту же поверхность
  триггеров. Глобальные горячие клавиши, фокус окна, наблюдение за файлами, HTTP-webhook,
  сон/пробуждение, включение и выключение дисплея, подключение и отключение устройств, блокировка
  и разблокировка сеанса, питание от сети или батареи, простой и активность пользователя, состояние
  сети.
* **Визуальные конвейеры действий** — перетаскивание действий с отменой и повтором, шаги ожидания
  с политикой тайм-аута, проверки в середине конвейера, ветвление If/Else, действия-переключатели,
  кулдауны правил и пожизненные или периодические квоты выполнения.
* **Режим сопутствующих процессов** — привязывайте вспомогательные инструменты, HUD или рекордеры
  к основной игре или приложению: они запускаются вместе с ней, контролируются во время работы и
  завершаются при её выходе.
* **Overlay Studio (HUD)** — WYSIWYG-оверлеи с GPU-ускорением: прицелы, телеметрические бейджи,
  графики, таймеры, медиа-виджеты, интерактивные элементы управления, размещение на нескольких
  мониторах и защита от захвата экрана `WDA_EXCLUDEFROMCAPTURE`.
* **Macro Studio** — запись клавиатуры и мыши через низкоуровневые хуки Windows, воспроизведение
  скан-кодами, совместимое с DirectInput, очеловеченное движение, повороты камеры через Raw Input,
  операции Run/Stop/Toggle и переиспользуемые пресеты.
* **Скрипты и автоматизация из CLI** — встроенный Lua 5.4.9 с API `ph`, действия PowerShell и Batch;
  авторство правил в JSON через `--rules-apply`/`--rules-sync` со схемой, шаблонами и справочником
  ([справочник CLI](docs/RULE_AUTHORING.md)); share-коды `SYN1:`, ярлыки `.lnk` на рабочем столе и
  `--run-rule`.
* **Управление системой и устройствами** — обрезка рабочего набора, приоритет и сродство CPU,
  переключение брандмауэра, действия с окнами, общая громкость и громкость приложений, баланс
  каналов, формат звука и переключение устройств воспроизведения и записи по умолчанию.
* **Наблюдаемость** — панель аналитики с KPI-карточками, поток логов в реальном времени и суточные
  ротируемые файлы логов с настраиваемой подробностью.
* **Установщик и портативные сборки** — нативный установщик Windows с деинсталлятором, ярлыком в
  меню «Пуск» и интеграцией `App Paths`, либо один портативный исполняемый файл. Обе формы
  обновляются сами через управляемую демоном проверку релизов GitHub с проверкой SHA-256,
  прогрессом в трее и необязательной автоустановкой.

## Установка

Требуется Windows 10 / 11 (x64).

- **Установщик** — `Synector-Setup-<version>-Windows-x64.exe` из
  [**релизов GitHub**](https://github.com/psyattack/Synector/releases/latest).
- **Портативная версия** — `Synector-v<version>-Windows-x64.zip`: распакуйте в любую папку и
  запустите `Synector.exe`.
- **Скрипт установки** — одна строка в PowerShell:
  `irm https://raw.githubusercontent.com/psyattack/Synector/main/installer/install.ps1 | iex`.

Каждый релиз также публикует `SHA256SUMS.txt`; сравните его с результатом
`Get-FileHash -Algorithm SHA256`, если проверяете загрузку вручную. Предупреждения SmartScreen,
повышенные права и вопросы об обновлениях разобраны в
[docs/FAQ.md](docs/FAQ.md#installation-and-updates) (на английском).

> [!IMPORTANT]
> **Права администратора**: Synector по умолчанию запускается с повышенными правами (отключить
> можно флагом `--no-elevate`). Это необходимо для:
> * **ETW ядра** — событий жизненного цикла процессов без опроса в реальном времени.
> * **Обхода UIPI** — аппаратной симуляции ввода в приложения и игры с повышенными правами.
> * **Управления процессами и автозапуска** — настройки памяти и сродства, а также тихого
>   автозапуска с наивысшими правами через Планировщик заданий.

> [!NOTE]
> При первом запуске демон создаёт задачу автозапуска в Планировщике заданий (настраивается в
> настройках).

## Разработка

Требуется свежий стабильный инструментарий Rust на Windows; текущим зависимостям
`eframe`/`egui` 0.37 нужен Rust **1.95 или новее**. Проект не фиксирует MSRV.

Процесс участия, стиль кода и правила документации описаны в [CONTRIBUTING.md](CONTRIBUTING.md)
(на английском).

```powershell
# Клонировать репозиторий
git clone https://github.com/psyattack/Synector.git
cd Synector

# Проверить сборку и запустить все модульные тесты
cargo check
cargo test

# Линт-проверка, используемая в CI (предупреждения считаются ошибками)
cargo clippy -- -D warnings

# Запуск в режиме разработки (поднимает демон и открывает GUI)
cargo run

# Только фоновый демон (режим трея)
cargo run -- --daemon

# GUI против уже запущенного демона
cargo run -- --gui

# Статус демона по именованному каналу
cargo run -- --status

# Оптимизированная release-сборка
cargo build --release
```

CI (`.github/workflows/ci.yml`) запускает `cargo check`, `cargo test` и `cargo clippy -- -D warnings`
на `windows-latest`.

### Стек

- **UI:** egui / eframe, egui_extras, egui-pop-phosphor (egui 0.37 fork of egui-phosphor)
- **Рантайм:** Tokio
- **Движок:** mlua (встроенный Lua 5.4.9), windows-rs
- **Данные:** Serde / Serde JSON, schemars
- **Сеть:** reqwest + rustls
- **Сжатие и хеширование:** brotli, flate2, zip, sha2

### Структура проекта

<details>
<summary>Разбивка по файлам</summary>

```
Synector/
├── .github/
│   ├── ISSUE_TEMPLATE/                    # Формы баг-репортов и предложений
│   ├── workflows/
│   │   ├── ci.yml                         # Check, test и clippy на windows-latest
│   │   └── release.yml                    # Портативный zip, установщик Inno Setup и релиз GitHub
│   ├── dependabot.yml                     # Еженедельные обновления cargo и GitHub Actions
│   └── PULL_REQUEST_TEMPLATE.md           # Чек-лист pull request
├── assets/
│   ├── fonts/                             # Встроенные вариативные шрифты (Archivo, GeistMono, Inter) + тексты OFL
│   ├── screenshots/                       # Скриншоты для документации
│   ├── app.ico                            # Иконка приложения Win32
│   ├── app.png                            # Иконка RGBA высокого разрешения
│   ├── app.svg                            # Векторный логотип
│   └── social-preview.png                 # Социальное превью GitHub 1280x640
├── docs/
│   ├── README.md                          # Карта документации
│   ├── ARCHITECTURE.md                    # Поведение подсистем и соглашения проектирования
│   ├── RULE_AUTHORING.md                  # Справочник по авторству правил из CLI
│   ├── EXAMPLES.md                        # Готовые рецепты правил
│   └── FAQ.md                             # Вопросы пользователей и устранение неполадок
├── installer/
│   ├── install.cmd                        # Консольный загрузчик (меню: Setup / Portable / Uninstall)
│   ├── install.ps1                        # Загрузка релиза, проверка SHA-256, Setup/Portable/Uninstall
│   └── synector.iss                       # Скрипт Inno Setup (компилируется только в CI)
├── src/
│   ├── daemon/                            # Фоновый демон
│   │   ├── actions.rs                     # Обработчики всех вариантов Action
│   │   ├── discord_rpc.rs                 # Клиент Discord Rich Presence через \\.\pipe\discord-ipc-*
│   │   ├── engine.rs                      # Планировщик правил, квоты/кулдауны, активные выполнения, аналитика
│   │   ├── file_watcher.rs                # Адаптивный наблюдатель за папками и файлами
│   │   ├── hotkeys.rs                     # Монитор глобальных горячих клавиш (обычный, удержание, повтор)
│   │   ├── ipc_server.rs                  # Слушатель именованного канала и воркер соединений
│   │   ├── lua_runner.rs                  # Встроенный Lua 5.4.9 с API `ph`
│   │   ├── overlay_manager.rs             # Жизненный цикл оверлеев, калибровка и размещение
│   │   ├── pe_mask.rs                     # Замаскированный образ демона WindowsServiceHost.exe
│   │   ├── system_events.rs               # Сон/пробуждение, дисплей, устройства, сеанс, питание, простой, сеть
│   │   ├── telemetry.rs                   # CPU/RAM/сеть/батарея/микрофон, визуализатор клавиш, медиа
│   │   ├── tray.rs                        # Иконка трея Win32, меню, уведомления и прогресс обновления
│   │   ├── watcher.rs                     # Гибридный наблюдатель процессов ETW / Toolhelp32
│   │   └── webhook_server.rs              # Локальный HTTP-сервер триггеров (127.0.0.1:48123)
│   ├── gui/                               # Клиент egui
│   │   ├── components/                    # Общие виджеты и рендереры
│   │   ├── views/                         # Экраны и панели приложения
│   │   ├── components.rs                  # Экспорт модулей компонентов
│   │   ├── macro_recorder.rs              # Запись макросов через низкоуровневые хуки и Raw Input
│   │   ├── overlay_window.rs              # Отдельное безрамочное окно оверлея с GPU
│   │   ├── state.rs                       # Состояние GUI, черновики, undo/redo, синхронизация IPC
│   │   ├── theme.rs                       # Токены дизайна и палитры Dark/Light/Chocolate
│   │   └── views.rs                       # Экспорт модулей экранов
│   ├── i18n/                              # Каталоги локализации
│   │   ├── en.rs                          # Английские строки
│   │   └── ru.rs                          # Русские строки
│   ├── ipc/                               # Межпроцессное взаимодействие
│   │   ├── client.rs                      # Клиент именованного канала
│   │   └── protocol.rs                    # IpcRequest/IpcResponse и DaemonState
│   ├── autorun.rs                         # Интеграция автозапуска через Планировщик заданий
│   ├── cli.rs                             # CLI авторства правил (validate, apply, sync, share, ...)
│   ├── config.rs                          # Версионированное раздельное JSON-хранилище и кеши чтения
│   ├── daemon.rs                          # Точка входа демона, циклы и подключение подсистем
│   ├── gui.rs                             # Точка входа GUI и цикл приложения eframe
│   ├── i18n.rs                            # Реестр языков и интерфейс каталога `Strings`
│   ├── ipc.rs                             # `PIPE_NAME` и реэкспорты IPC
│   ├── logging.rs                         # Суточные логи, уровни, хранение и перехват паник
│   ├── main.rs                            # Диспетчер CLI (оверлей, правила, статус, повышение прав, уровни)
│   ├── models.rs                          # Доменная модель (Rule, Trigger, Action, условия, оверлеи)
│   ├── sharing.rs                         # Share-коды SYN1: (Brotli + Base64, контрольная сумма, границы)
│   ├── shortcut.rs                        # Ярлыки Windows (.lnk) для запуска правил
│   ├── single_instance.rs                 # Мьютексы единственного экземпляра и фокус окна
│   └── updater.rs                         # Проверка релизов GitHub, SHA-256, самообновление
├── build.rs                               # Компиляция ресурсов Windows (иконка, версия, манифест)
├── CHANGELOG.md                           # История релизов
├── CODE_OF_CONDUCT.md                     # Стандарты сообщества
├── CONTRIBUTING.md                        # Сборка, стиль и процесс pull request
├── Cargo.toml                             # Манифест Rust и зависимости
├── LICENSE                                # Лицензия MIT
├── README.md                              # Английская версия этого файла
├── README.ru.md                           # Этот файл
├── SECURITY.md                            # Политика сообщения об уязвимостях
└── THIRD-PARTY-NOTICES.md                 # Лицензии и атрибуции третьих сторон
```

</details>

Как части ведут себя и взаимодействуют — два уровня, поток данных, движок правил, оверлеи,
симуляция ввода и мониторинг процессов — описано в
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (на английском).

## Конфигурация

Все пользовательские файлы лежат в **`%LOCALAPPDATA%\Synector\`**. Переменная
`SYNECTOR_CONFIG_DIR` переносит весь каталог для портативных или изолированных запусков.

| Файл / папка | Содержимое |
|---|---|
| `settings.json` | Геометрия окна, видимость трея, маскировка демона, язык, тема и параметры обновлений |
| `rules.json` | Определения правил автоматизации (счётчики времени выполнения записываются обнулёнными) |
| `library.json` | Пресеты оверлеев и макросов |
| `runtime.json` | Счётчики выполнений правил, время последнего запуска и якоря пользовательских окон квот |
| `logs\` | Суточные диагностические логи, хранение 7 дней и лимит 8 МиБ на файл |

Запись атомарна (временный файл → сброс → переименование), и каждая секция несёт версию схемы.
Секция, которую не удалось разобрать, помещается в карантин как `<name>.corrupt-<timestamp>`, после
чего заменяется значениями по умолчанию, — файл, отредактированный вручную, не уничтожается молча.
Пока демон доступен, только он выполняет запись; GUI и CLI переходят к прямой записи файлов лишь
при недоступности IPC. Делайте резервные копии и редактируйте эти файлы при остановленном Synector.
Путь записи, кеши чтения и правила объединения записей описаны в
[docs/ARCHITECTURE.md#configuration-storage](docs/ARCHITECTURE.md#configuration-storage).

## Авторство правил из CLI

Правила — это обычный JSON, который можно проверять, применять, заменять, экспортировать,
передавать и сбрасывать без открытия GUI:

```powershell
Synector.exe --rules-schema > schema.json
Synector.exe --rules-template schedule:play_sound > rule.json
Synector.exe --rules-validate rule.json
Synector.exe --rules-apply rule.json
Synector.exe --rules-sync rule.json --dry-run --json
Synector.exe --rules-share my_rule
```

Полный справочник команд, семантику квот, форматы JSON-отчётов и поведение share-кодов `SYN1:`
смотрите в [docs/RULE_AUTHORING.md](docs/RULE_AUTHORING.md), а готовые рецепты — в
[docs/EXAMPLES.md](docs/EXAMPLES.md).

## Документация

| Документ | Для кого | Содержимое |
|---|---|---|
| [docs/README.md](docs/README.md) | Все | Карта документации и правила |
| [docs/RULE_AUTHORING.md](docs/RULE_AUTHORING.md) | Авторы правил | Справочник CLI, формат JSON, квоты, share-коды |
| [docs/EXAMPLES.md](docs/EXAMPLES.md) | Авторы правил | Готовые к применению рецепты |
| [docs/FAQ.md](docs/FAQ.md) | Пользователи | Частые вопросы и устранение неполадок |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Разработчики | Подсистемы, поток данных и обоснование решений |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Участники | Сборка, тесты, стиль и процесс pull request |
| [SECURITY.md](SECURITY.md) | Исследователи безопасности | Политика приватного сообщения об уязвимостях |
| [CHANGELOG.md](CHANGELOG.md) | Все | История релизов |

Документация в `docs/` и большинство файлов сообщества написаны на английском.

## Лицензия

Проект распространяется по [лицензии MIT](LICENSE). Компоненты третьих сторон:
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Благодарности

- **Идея UI/UX:** [ArtCraft Launcher](https://github.com/storytold/craft-launcher)

## Поддержка и участие

- **Ошибки и предложения:** создайте issue в
  [**GitHub Issues**](https://github.com/psyattack/Synector/issues/new/choose). Укажите версию
  Synector, `winver`, запускалось ли приложение с повышенными правами, шаги воспроизведения и
  относящиеся к делу строки лога без секретов.
- **Вопросы и неполадки:** см. [docs/FAQ.md](docs/FAQ.md).
- **Участие в разработке:** процесс сборки, стиль и ревью описаны в
  [CONTRIBUTING.md](CONTRIBUTING.md); стандарты сообщества — в
  [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
- **Безопасность:** сообщайте приватно, как описано в [SECURITY.md](SECURITY.md). Не открывайте
  публичный issue об уязвимости.
