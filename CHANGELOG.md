# Changelog

Формат основан на [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/), версии следуют [Semantic Versioning](https://semver.org/lang/ru/).

## [Unreleased]

## [0.1.0] - 2026-11-19

### Added

- Единое IR документов с блоками `Heading`, `Paragraph`, `Table`, `Image`, `PageBreak`, `SheetBreak` и локациями `Loc`.
- PDF-бэкенд на `pdf_oxide` с резервным `pdf-extract`, определением заголовков и опциональным извлечением изображений.
- DOCX-бэкенд на `office_oxide` IR.
- Бэкенд таблиц на `calamine` для XLSX, XLSM, XLS и ODS.
- Бэкенд изображений с размерами, EXIF и OCR.
- OCR-бэкенды `ocrs`, `tesseract` и `vision` с выбором в рантайме.
- Токенный чанкинг на `cl100k_base` с границами по заголовкам, страницам и листам.
- CLI `aitools` с выводом Markdown, JSON и чанков.
- MCP-сервер `aitools-mcp` с инструментами `extract_file`, `chunk_file`, `request_access`, `extract_document`, `chunk_document` и `list_formats`.
- Кэш результатов по SHA-256 с инвалидацией по `mtime` и размеру файла.
- Launcher `scripts/aitools-mcp-sandbox.sh` на `bwrap` с доступом только к проекту и `grant_base`.
- Гранты на отдельные каталоги и файлы, хранящиеся отдельно для каждого проекта.
- Журнал обращений и грантов в `<root>/.aitools/audit.log`.
- Репозиторная обвязка: лицензии MIT и Apache-2.0, CI, шаблоны issue и pull request, Makefile, настройки `rustfmt`, `clippy` и `editorconfig`.
