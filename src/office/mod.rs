//! Office backend: DOCX via `office_oxide` IR, spreadsheets via `calamine`.

use std::io::Cursor;

use calamine::{Data, Reader, open_workbook_auto_from_rs};
use office_oxide::Document;
use office_oxide::format::DocumentFormat;
use office_oxide::ir::{Element, InlineContent, inline_to_text};

use crate::error::{Error, Result};
use crate::extract::{ExtractOptions, Extractor};
use crate::model::{
    Block, Document as ToolkitDocument, FileKind, Loc, Source, Span, Style, Warning,
};

pub struct DocxExtractor;

impl Extractor for DocxExtractor {
    fn kinds(&self) -> &'static [FileKind] {
        &[FileKind::Docx]
    }

    fn extract(
        &self,
        source: &Source,
        opts: &ExtractOptions,
        out: &mut ToolkitDocument,
    ) -> Result<()> {
        let document =
            Document::from_reader(Cursor::new(source.bytes.clone()), DocumentFormat::Docx)
                .map_err(|error| Error::Other(format!("DOCX parse failed: {error}")))?;
        let ir = document.to_ir();

        if ir.metadata.has_macros {
            out.warnings.push(Warning::new(
                "macros",
                "document contains macros; they were not executed or extracted",
            ));
        }

        for (index, section) in ir.sections.iter().enumerate() {
            if index > 0 {
                out.blocks.push(Block::PageBreak);
            }
            let loc = Loc {
                page: None,
                sheet: None,
                section: Some(index as u32 + 1),
                bbox: None,
            };
            for element in &section.elements {
                convert_element(element, &loc, opts, out);
            }
        }

        if out.blocks.is_empty() {
            out.warnings
                .push(Warning::new("empty_document", "no DOCX content found"));
        }
        out.warnings
            .push(Warning::new("office_strategy", "office_oxide IR"));

        Ok(())
    }
}

fn convert_element(element: &Element, loc: &Loc, opts: &ExtractOptions, out: &mut ToolkitDocument) {
    match element {
        Element::Heading(heading) => out.blocks.push(Block::Heading {
            level: heading.level.clamp(1, 6),
            text: inline_to_text(&heading.content),
            loc: *loc,
        }),
        Element::Paragraph(paragraph) => {
            let spans = inline_spans(&paragraph.content);
            if !spans.is_empty() {
                out.blocks.push(Block::Paragraph { spans, loc: *loc });
            }
        }
        Element::Table(table) => {
            let rows: Vec<Vec<String>> = table
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|cell| elements_text(&cell.content))
                        .collect()
                })
                .collect();
            if !rows.is_empty() {
                out.blocks.push(Block::Table { rows, loc: *loc });
            }
        }
        Element::List(list) => {
            for item in &list.items {
                let text = elements_text(&item.content);
                if !text.trim().is_empty() {
                    let mut spans = vec![Span::plain(if list.ordered { "1. " } else { "- " })];
                    spans.extend(inline_text_spans(&text));
                    out.blocks.push(Block::Paragraph { spans, loc: *loc });
                }
                if let Some(nested) = &item.nested {
                    convert_list(nested, loc, out);
                }
            }
        }
        Element::Image(image) => out.blocks.push(Block::Image {
            data: if opts.include_images {
                image.data.clone()
            } else {
                None
            },
            mime: image
                .format
                .as_ref()
                .map(|format| format.content_type().to_owned())
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            alt: image.alt_text.clone(),
            loc: *loc,
        }),
        Element::CodeBlock(code) => out.blocks.push(Block::Paragraph {
            spans: vec![Span {
                text: code.content.clone(),
                style: Style {
                    code: true,
                    ..Style::default()
                },
            }],
            loc: *loc,
        }),
        Element::PageBreak => out.blocks.push(Block::PageBreak),
        Element::TextBox(text_box) => {
            for element in &text_box.content {
                convert_element(element, loc, opts, out);
            }
        }
        Element::Footnote(note) | Element::Endnote(note) => {
            for element in &note.content {
                convert_element(element, loc, opts, out);
            }
        }
        Element::ThematicBreak | Element::ColumnBreak | Element::Shape(_) => {}
        _ => {}
    }
}

fn convert_list(list: &office_oxide::ir::List, loc: &Loc, out: &mut ToolkitDocument) {
    for item in &list.items {
        let text = elements_text(&item.content);
        if !text.trim().is_empty() {
            out.blocks.push(Block::Paragraph {
                spans: vec![Span::plain(format!("  - {text}"))],
                loc: *loc,
            });
        }
        if let Some(nested) = &item.nested {
            convert_list(nested, loc, out);
        }
    }
}

fn inline_spans(content: &[InlineContent]) -> Vec<Span> {
    let mut spans = Vec::new();
    for item in content {
        match item {
            InlineContent::Text(text) if !text.text.is_empty() => spans.push(Span {
                text: text.text.clone(),
                style: Style {
                    bold: text.bold,
                    italic: text.italic,
                    code: false,
                },
            }),
            InlineContent::LineBreak => spans.push(Span::plain(" ")),
            _ => {}
        }
    }
    spans
}

fn inline_text_spans(text: &str) -> Vec<Span> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![Span::plain(text)]
    }
}

fn elements_text(elements: &[Element]) -> String {
    let mut parts = Vec::new();
    for element in elements {
        match element {
            Element::Heading(heading) => parts.push(inline_to_text(&heading.content)),
            Element::Paragraph(paragraph) => parts.push(inline_to_text(&paragraph.content)),
            Element::Table(table) => {
                for row in &table.rows {
                    let cells: Vec<String> = row
                        .cells
                        .iter()
                        .map(|cell| elements_text(&cell.content))
                        .collect();
                    parts.push(cells.join(" | "));
                }
            }
            Element::List(list) => {
                for item in &list.items {
                    parts.push(format!("- {}", elements_text(&item.content)));
                }
            }
            Element::Image(image) => {
                if let Some(alt) = &image.alt_text {
                    parts.push(alt.clone());
                }
            }
            Element::CodeBlock(code) => parts.push(code.content.clone()),
            Element::TextBox(text_box) => parts.push(elements_text(&text_box.content)),
            Element::Footnote(note) | Element::Endnote(note) => {
                parts.push(elements_text(&note.content));
            }
            Element::PageBreak
            | Element::ThematicBreak
            | Element::ColumnBreak
            | Element::Shape(_) => {}
            _ => {}
        }
    }
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct XlsxExtractor;

impl Extractor for XlsxExtractor {
    fn kinds(&self) -> &'static [FileKind] {
        &[FileKind::Xlsx]
    }

    fn extract(
        &self,
        source: &Source,
        _opts: &ExtractOptions,
        out: &mut ToolkitDocument,
    ) -> Result<()> {
        let mut workbook = open_workbook_auto_from_rs(Cursor::new(source.bytes.clone()))
            .map_err(|error| Error::Other(format!("spreadsheet parse failed: {error}")))?;

        for (index, name) in workbook.sheet_names().iter().enumerate() {
            let range = match workbook.worksheet_range(name) {
                Ok(range) => range,
                Err(error) => {
                    out.warnings.push(Warning::new(
                        "sheet_failed",
                        format!("sheet '{name}': {error}"),
                    ));
                    continue;
                }
            };

            out.blocks.push(Block::SheetBreak { name: name.clone() });
            let rows = trim_rows(
                range
                    .rows()
                    .map(|row| row.iter().map(cell_to_string).collect())
                    .collect(),
            );

            if rows.is_empty() {
                out.warnings.push(Warning::new(
                    "empty_sheet",
                    format!("sheet '{name}' has no cell values"),
                ));
                continue;
            }

            out.blocks.push(Block::Table {
                rows,
                loc: Loc {
                    page: None,
                    sheet: Some(index as u32),
                    section: None,
                    bbox: None,
                },
            });
        }

        out.warnings
            .push(Warning::new("office_strategy", "calamine"));

        Ok(())
    }
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(value) | Data::DateTimeIso(value) | Data::DurationIso(value) => value.clone(),
        Data::Int(value) => value.to_string(),
        Data::Float(value) => {
            if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                value.to_string()
            }
        }
        Data::Bool(value) => value.to_string(),
        Data::DateTime(value) => value.to_string(),
        Data::Error(value) => format!("#{value:?}"),
    }
}

fn trim_rows(mut rows: Vec<Vec<String>>) -> Vec<Vec<String>> {
    while rows
        .last()
        .is_some_and(|row| row.iter().all(|cell| cell.trim().is_empty()))
    {
        rows.pop();
    }
    if rows.is_empty() {
        return rows;
    }

    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut keep = columns;
    while keep > 0
        && rows
            .iter()
            .all(|row| row.get(keep - 1).is_none_or(|cell| cell.trim().is_empty()))
    {
        keep -= 1;
    }
    for row in &mut rows {
        row.truncate(keep);
    }
    rows
}
