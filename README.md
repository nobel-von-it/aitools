# aitools

Универсальный набор утилит для извлечения текста и структуры из PDF, DOCX, XLSX, изображений и текстовых файлов. Все форматы приводятся к единому промежуточному представлению (IR), которое можно отдать агенту в виде Markdown или JSON, а также разрезать на чанки по токенам с сохранением границ документа.

Проект состоит из Rust-библиотеки, CLI и MCP-сервера для stdio-транспорта. MCP-сервер можно запускать в песочнице bubblewrap с доступом только к каталогу проекта и явно разрешённым каталогам.

## Содержание

- [Возможности](#возможности)
- [Архитектура](#архитектура)
- [Установка](#установка)
- [CLI](#cli)
- [Библиотека](#библиотека)
- [Поддерживаемые форматы](#поддерживаемые-форматы)
- [OCR](#ocr)
- [Чанкинг](#чанкинг)
- [MCP-сервер](#mcp-сервер)
- [Конфигурация](#конфигурация)
- [Feature-флаги](#feature-флаги)
- [Тесты и проверки](#тесты-и-проверки)
- [Структура репозитория](#структура-репозитория)
- [Ограничения](#ограничения)
- [Участие в разработке](#участие-в-разработке)
- [Лицензия](#лицензия)

## Возможности

- Единое IR: `Document { source, blocks, warnings }`, блоки `Heading`, `Paragraph`, `Table`, `Image`, `PageBreak`, `SheetBreak`.
- Локации `Loc { page, sheet, section, bbox }` для ссылок на источник.
- Деградация вместо падения: частичный результат дополняется предупреждениями `Warning` с машинно-читаемым кодом.
- PDF: основной парсер `pdf_oxide`, резервный `pdf-extract`, определение заголовков по размеру шрифта, порядок чтения с учётом колонок, опциональное извлечение изображений.
- Office: DOCX через IR `office_oxide`; XLSX, XLSM, XLS и ODS через `calamine`.
- Изображения: размеры, MIME, EXIF и опциональный OCR.
- OCR с выбором бэкенда на каждый вызов: `off`, `auto`, `ocrs`, `tesseract`, `vision`.
- Токенный чанкинг на `cl100k_base` с границами по заголовкам, страницам и листам.
- CLI с выводом Markdown, JSON или чанков.
- MCP-сервер с файловыми и inline-инструментами, кэшем и ограничением доступа.
- Песочница bubblewrap и гранты на отдельные каталоги, хранящиеся отдельно для каждого проекта.

## Архитектура

```
Source { meta, bytes }
  -> detect (magic bytes, затем расширение)
  -> Registry (выбор Extractor по FileKind)
  -> Document { blocks, warnings }        единое IR
  -> render (Markdown | JSON)
  -> chunk (токены + структурные границы)
```

Ключевые типы:

```rust
pub struct Document {
    pub source: SourceMeta,
    pub blocks: Vec<Block>,
    pub warnings: Vec<Warning>,
}

pub enum Block {
    Heading { level: u8, text: String, loc: Loc },
    Paragraph { spans: Vec<Span>, loc: Loc },
    Table { rows: Vec<Vec<String>>, loc: Loc },
    Image { data: Option<Vec<u8>>, mime: String, alt: Option<String>, loc: Loc },
    PageBreak,
    SheetBreak { name: String },
}
```

`Registry::extract` проверяет лимиты до и после разбора. Лимиты по умолчанию:

| Лимит | Значение |
|---|---|
| `max_input_bytes` | 64 МиБ |
| `max_output_chars` | 2 000 000 символов |
| `max_pages` | 200 страниц или листов |

## Установка

Требования:

- Rust 1.85 или новее (edition 2024).
- Для OCR через Tesseract нужен системный бинарник `tesseract` в `PATH`.
- Для песочницы MCP нужен `bwrap` (пакет `bubblewrap`).
- Для чисто-Rust OCR нужны ONNX-модели `ocrs`, если используется этот бэкенд.

Сборка и установка:

```bash
git clone <repository-url> aitools
cd aitools

cargo build --release --all-features

cargo install --path . --features cli --bin aitools --root "$HOME/.local"
cargo install --path . --features mcp --bin aitools-mcp --root "$HOME/.local"
install -m 755 scripts/aitools-mcp-sandbox.sh "$HOME/.local/bin/aitools-mcp-sandbox"
```

Если OCR не нужен, вместо `--all-features` можно использовать набор по умолчанию: `pdf`, `office`, `image`, `markdown`.

## CLI

```bash
aitools README.md --format md
aitools report.pdf --format json
aitools scan.png --ocr auto --tesseract-lang rus+eng
aitools article.md --chunk --max-tokens 500 --overlap-tokens 50
```

Флаги:

| Флаг | Значение по умолчанию | Описание |
|---|---|---|
| `--format md\|json` | `md` | Формат вывода |
| `--chunk` | выключен | Вывести JSON-чанки вместо документа |
| `--ocr off\|auto\|ocrs\|tesseract\|vision` | `off` | OCR-бэкенд для изображений |
| `--include-images` | выключен | Включить сырые байты изображений в JSON |
| `--tesseract-lang LANG` | `eng` | Языки Tesseract, например `rus+eng` |
| `--max-tokens N` | `800` | Максимальный размер чанка |
| `--overlap-tokens N` | `80` | Перекрытие соседних чанков |
| `--max-output-chars N` | `2000000` | Лимит символов результата |
| `--max-pages N` | `200` | Лимит страниц или листов |
| `--help`, `-h` | | Справка |

## Библиотека

```rust
use std::sync::Arc;

use aitools::chunk::{ChunkOptions, chunk};
use aitools::render::{RenderFormat, render};
use aitools::{ExtractOptions, OcrMode, Registry, Source};

fn main() -> aitools::Result<()> {
    let source = Source::from_path("report.pdf")?;
    let document = Registry::default().extract(source, &ExtractOptions::default())?;

    let markdown = render(&document, RenderFormat::Markdown)?;
    println!("{markdown}");

    let chunks = chunk(&document, &ChunkOptions::default())?;
    println!("{} chunks", chunks.len());

    let _options = ExtractOptions {
        ocr: OcrMode::Auto,
        ..ExtractOptions::default()
    };
    Ok(())
}
```

Пользовательский экстрактор можно зарегистрировать в `Registry`:

```rust
use aitools::{ExtractOptions, Extractor, FileKind, Registry, Source};
use aitools::model::Document;

struct MyExtractor;

impl Extractor for MyExtractor {
    fn kinds(&self) -> &'static [FileKind] {
        &[FileKind::Text]
    }

    fn extract(
        &self,
        source: &Source,
        _options: &ExtractOptions,
        out: &mut Document,
    ) -> aitools::Result<()> {
        out.warnings.push(aitools::Warning::new("demo", source.meta.name.clone()));
        Ok(())
    }
}

let mut registry = Registry::default();
registry.register(MyExtractor);
```

## Поддерживаемые форматы

| Формат | Определение | Поведение |
|---|---|---|
| PDF | magic bytes, `.pdf` | Текст по страницам, заголовки, bbox, `PageBreak`; fallback на `pdf-extract` |
| DOCX | magic bytes, `.docx` | Заголовки, абзацы, таблицы, списки, код, текстовые блоки, сноски, изображения |
| XLSX, XLSM, XLS, ODS | magic bytes, расширение | `SheetBreak` и `Table` на каждый лист через `calamine` |
| PNG, JPEG, GIF, WebP, TIFF | magic bytes, расширение | Размеры, MIME, EXIF, опциональный OCR |
| TXT, LOG, CSV, JSON, XML, YAML, YML | magic bytes, расширение | Абзацы, разбиение по пустым строкам |
| Markdown, MD | расширение | Заголовки и абзацы |
| SVG | расширение | Определяется как изображение; декодирование зависит от бэкенда |

Определение формата сначала идёт по magic bytes, затем по расширению. Неизвестный формат возвращает `Error::Unsupported(FileKind::Unknown)`.

Предупреждения, которые может вернуть документ:

| Код | Причина |
|---|---|
| `encrypted` | PDF зашифрован, извлечено только доступное без пароля |
| `pdf_fallback` | Основной PDF-парсер не справился |
| `pdf_strategy` | Какой парсер фактически использован |
| `no_text_layer` | На странице PDF нет текстового слоя |
| `page_limit` | Документ обрезан по `max_pages` |
| `no_pages` | Основной парсер не нашёл страниц |
| `office_strategy` | Какой office-бэкенд использован |
| `macros` | В DOCX есть макросы, они не исполнялись |
| `empty_document` | Извлечённый документ пуст |
| `sheet_failed` | Лист не удалось прочитать |
| `empty_sheet` | На листе нет значений |
| `unknown_image_format` | Изображение не удалось декодировать |
| `ocr_backend` | Каким бэкендом распознан текст |
| `ocr_disabled` | OCR выключен |
| `ocr_empty` | Бэкенд не вернул текст |
| `ocrs_model_missing` | Не заданы ONNX-модели |
| `tesseract_unavailable` | Не найден бинарник `tesseract` |
| `vision_backend_missing` | Не передан `VisionOcr` |
| `no_ocr_backend` | В режиме `auto` нет доступных бэкендов |

## OCR

`OcrMode` выбирается на каждый вызов:

| Режим | Поведение |
|---|---|
| `Off` | OCR не выполняется |
| `Auto` | Пробует `ocrs`, затем `tesseract`, затем `vision` |
| `Ocrs` | Чисто-Rust OCR через `ocrs` и `rten` |
| `Tesseract` | Внешний бинарник `tesseract` |
| `Vision` | Пользовательская реализация трейта `VisionOcr` |

`ocrs` требует две ONNX-модели:

```bash
export AITOOLS_OCRS_DETECTION_MODEL=/path/to/detection.onnx
export AITOOLS_OCRS_RECOGNITION_MODEL=/path/to/recognition.onnx
```

Эти же пути можно передать через поля `OcrConfig::ocrs_detection_model` и `OcrConfig::ocrs_recognition_model`.

Для `Vision` нужно передать реализацию трейта в `ExtractOptions::vision`:

```rust
use std::sync::Arc;

use aitools::{ExtractOptions, OcrMode, VisionOcr};

struct MyVision;

impl VisionOcr for MyVision {
    fn recognize(&self, _image: &[u8], _mime: &str) -> aitools::Result<String> {
        Ok("распознанный текст".to_owned())
    }
}

let options = ExtractOptions {
    ocr: OcrMode::Vision,
    vision: Some(Arc::new(MyVision)),
    ..ExtractOptions::default()
};
```

Ошибка OCR не приводит к потере документа: изображение остаётся в IR, а причина попадает в `warnings`.

## Чанкинг

Чанкинг использует `cl100k_base` из `tiktoken-rs` и старается резать документ по смысловым границам: заголовкам, разрывам страниц и листам. Каждый чанк содержит:

```rust
pub struct Chunk {
    pub index: usize,
    pub text: String,
    pub tokens: usize,
    pub headings: Vec<String>,
    pub locs: Vec<Loc>,
}
```

Параметры по умолчанию: `max_tokens = 800`, `overlap_tokens = 80`. Перекрытие формируется из последних блоков предыдущего чанка и сбрасывается на принудительных границах.

## MCP-сервер

MCP-сервер работает по stdio и отдаёт инструменты для извлечения, чанкинга, запроса доступа и просмотра текущей области видимости.

### Инструменты

| Инструмент | Описание |
|---|---|
| `extract_file` | Путь в проекте или гранте -> Markdown или JSON (`format`, `ocr`, `include_images`) |
| `chunk_file` | Путь в проекте или гранте -> JSON-чанки (`max_tokens`, `overlap_tokens`) |
| `request_access` | Запросить доступ к одному каталогу или файлу вне текущей области |
| `extract_document` | base64-байты и имя файла -> Markdown или JSON |
| `chunk_document` | base64-байты и имя файла -> JSON-чанки |
| `list_formats` | Поддерживаемые форматы, root, grant base и активные гранты |

Результаты кэшируются по SHA-256, кэш ограничен 64 записями. Для файловых инструментов в ключ входят канонический путь, `mtime` и размер, поэтому изменение файла автоматически инвалидирует кэш.

### Модель доступа

- `root` — каталог проекта, доступный всегда. По умолчанию это текущий рабочий каталог процесса.
- `grant_base` — максимальная граница, внутри которой можно запрашивать гранты. По умолчанию `~/Git`, если проект находится внутри него, иначе родительский каталог проекта.
- Гранты хранятся в `<root>/.aitools/grants.json`, то есть отдельно для каждого проекта. Сервер из другого проекта не видит эти пути.
- Разрешить весь `grant_base` целиком нельзя: `request_access` требует конкретный подкаталог или файл.
- `request_access` должен быть настроен в клиенте как `ask`, чтобы пользователь увидел точный путь и причину до выполнения.
- Все обращения и гранты пишутся в `<root>/.aitools/audit.log` с результатом `true` или `false`.
- `--no-inline` отключает `extract_document` и `chunk_document`, чтобы нельзя было обойти проверку пути через base64.

### Песочница

Скрипт `scripts/aitools-mcp-sandbox.sh` запускает сервер через `bwrap`:

- `~/Git` монтируется только для чтения;
- каталог проекта монтируется для чтения и записи, чтобы `request_access` мог сохранить грант;
- `/usr`, `/bin`, `/lib`, `/lib64`, `/etc` и `~/.local/bin` монтируются только для чтения;
- `/proc`, `/dev` и временный `/tmp` создаются внутри namespace;
- сеть сохраняется, чтобы работал `Vision`-бэкенд.

Launcher читает переменные `AITOOLS_PROJECT` (по умолчанию `$PWD`), `AITOOLS_BIN` (по умолчанию `~/.local/bin/aitools-mcp`) и `AITOOLS_GRANT_BASE`.

### OpenCode

```jsonc
// ~/.config/opencode/opencode.json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "aitools": {
        "type": "local",
        "command": ["/home/USER/.local/bin/aitools-mcp-sandbox"]
      }
    }
  },
  "permissions": [
    { "action": "aitools_request_access", "resource": "*", "effect": "ask" },
    { "action": "edit", "resource": "~/.config/opencode/opencode.json", "effect": "deny" },
    { "action": "edit", "resource": "~/.local/bin/aitools-mcp-sandbox", "effect": "deny" },
    { "action": "edit", "resource": "~/.local/bin/aitools-mcp", "effect": "deny" },
    { "action": "edit", "resource": "**/.aitools/grants.json", "effect": "deny" }
  ]
}
```

Проверка:

```bash
opencode mcp list
opencode debug config
```

### Zed

```json
// ~/.config/zed/settings.json
{
  "context_servers": {
    "aitools": {
      "source": "custom",
      "command": "/home/USER/.local/bin/aitools-mcp-sandbox",
      "args": [],
      "env": {}
    }
  }
}
```

В Zed нет нативного `ask` для произвольного MCP-инструмента, поэтому грант выдаётся явной просьбой пользователя в диалоге. Песочница при этом продолжает ограничивать процесс сервера.

### Границы безопасности

Песочница ограничивает процесс MCP-сервера, а не агента. У агента может оставаться `shell` с правами пользователя, поэтому он способен читать файлы напрямую, минуя MCP. Правила `permissions` закрывают редактирование конфигурации, launcher'а, бинарника и файла грантов, но не заменяют изоляцию самого агента. Для полной гарантии запускайте клиент в контейнере или под отдельным пользователем.

## Конфигурация

Переменные окружения:

| Переменная | Назначение |
|---|---|
| `AITOOLS_ROOT` | Корень проекта для MCP-сервера |
| `AITOOLS_GRANT_BASE` | Максимальная граница для грантов |
| `AITOOLS_GRANTS` | Путь к файлу грантов |
| `AITOOLS_NO_INLINE=1` | Отключить base64-инструменты |
| `AITOOLS_AUDIT_LOG` | Путь к журналу обращений |
| `AITOOLS_OCRS_DETECTION_MODEL` | ONNX-модель детекции текста для `ocrs` |
| `AITOOLS_OCRS_RECOGNITION_MODEL` | ONNX-модель распознавания текста для `ocrs` |
| `AITOOLS_PROJECT` | Проект для launcher'а (по умолчанию `$PWD`) |
| `AITOOLS_BIN` | Путь к бинарнику `aitools-mcp` для launcher'а |

## Feature-флаги

| Флаг | Назначение |
|---|---|
| `pdf` | PDF-бэкенд (`pdf_oxide`, `pdf-extract`) |
| `office` | DOCX и таблицы (`office_oxide`, `calamine`) |
| `image` | Изображения и EXIF (`image`, `kamadak-exif`) |
| `markdown` | Поддержка Markdown |
| `ocr-ocrs` | Чисто-Rust OCR (`ocrs`, `rten`) |
| `ocr-tesseract` | Совместимый флаг, подключает `image`; сам Tesseract вызывается при наличии бинарника |
| `ocr-vision` | Совместимый флаг, подключает `image`; vision-бэкенд внедряется через трейт |
| `cli` | Бинарник `aitools` |
| `mcp` | Бинарник `aitools-mcp` |

По умолчанию включены `pdf`, `office`, `image` и `markdown`.

## Тесты и проверки

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps --all-features
```

Или через Makefile:

```bash
make check
```

Тесты покрывают текстовый пайплайн, PDF, DOCX, XLSX, изображения, vision-OCR, чанкинг, лимиты и изоляцию путей MCP-сервера.

## Структура репозитория

```
.
├── Cargo.toml
├── README.md
├── LICENSE-MIT
├── LICENSE-APACHE
├── CONTRIBUTING.md
├── SECURITY.md
├── CHANGELOG.md
├── Makefile
├── rust-toolchain.toml
├── rustfmt.toml
├── clippy.toml
├── .editorconfig
├── .gitattributes
├── .github/
│   ├── workflows/ci.yml
│   ├── ISSUE_TEMPLATE/
│   └── PULL_REQUEST_TEMPLATE.md
├── scripts/
│   └── aitools-mcp-sandbox.sh
├── src/
│   ├── lib.rs
│   ├── model.rs
│   ├── error.rs
│   ├── extract.rs
│   ├── text.rs
│   ├── render.rs
│   ├── chunk.rs
│   ├── pdf/mod.rs
│   ├── office/mod.rs
│   ├── images/mod.rs
│   ├── ocr/mod.rs
│   └── bin/
│       ├── aitools.rs
│       └── aitools_mcp.rs
└── tests/
    ├── pipeline.rs
    ├── pdf.rs
    ├── office.rs
    ├── images.rs
    └── chunk.rs
```

## Ограничения

- PDF OCR не выполняется автоматически. Если у PDF нет текстового слоя, документ вернёт предупреждение `no_text_layer`; для сканов нужно сначала преобразовать страницы в изображения или использовать внешний OCR.
- Формулы в таблицах не вычисляются: `calamine` возвращает сохранённые значения.
- Макросы DOCX не исполняются и не извлекаются.
- `bbox` заполняется только для PDF.
- SVG определяется как изображение, но не всякий бэкенд умеет его декодировать.
- DOCX-изображения и PDF-изображения попадают в результат только при `include_images`.
- MCP-песочница не защищает от самого агента, у которого есть прямой доступ к файловой системе.

## Участие в разработке

Правила и локальные проверки описаны в [CONTRIBUTING.md](CONTRIBUTING.md). Перед отправкой изменений выполните `make check`.

## Лицензия

Проект распространяется по двойной лицензии MIT или Apache-2.0. Вы можете использовать его на условиях любой из них.

- [LICENSE-MIT](LICENSE-MIT)
- [LICENSE-APACHE](LICENSE-APACHE)
