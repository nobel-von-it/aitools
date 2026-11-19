#![cfg(feature = "office")]

use aitools::extract::{ExtractOptions, Registry};
use aitools::model::{Block, Source, SourceMeta};

fn source(name: &str, bytes: Vec<u8>) -> Source {
    Source {
        meta: SourceMeta {
            name: name.to_owned(),
            mime: "application/octet-stream".to_owned(),
            size_bytes: bytes.len() as u64,
        },
        bytes,
    }
}

#[test]
fn xlsx_becomes_sheets_and_tables() {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "Name").unwrap();
    sheet.write_string(0, 1, "Role").unwrap();
    sheet.write_string(1, 0, "Ada").unwrap();
    sheet.write_string(1, 1, "Engineer").unwrap();

    let other = workbook.add_worksheet();
    other.write_string(0, 0, "Note").unwrap();
    other.write_string(1, 0, "Second sheet").unwrap();

    let bytes = workbook.save_to_buffer().unwrap();
    let document = Registry::default()
        .extract(source("book.xlsx", bytes), &ExtractOptions::default())
        .unwrap();

    let sheet_names: Vec<_> = document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::SheetBreak { name } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(sheet_names, vec!["Sheet1", "Sheet2"]);

    let first_table = document
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::Table { rows, .. } => Some(rows),
            _ => None,
        })
        .unwrap();
    assert_eq!(first_table[0], vec!["Name", "Role"]);
    assert_eq!(first_table[1], vec!["Ada", "Engineer"]);
}

#[test]
fn docx_becomes_blocks() {
    let docx = docx_rs::Docx::new()
        .add_paragraph(
            docx_rs::Paragraph::new().add_run(docx_rs::Run::new().add_text("Office heading")),
        )
        .add_paragraph(
            docx_rs::Paragraph::new().add_run(docx_rs::Run::new().add_text("Body text.")),
        );

    let mut cursor = std::io::Cursor::new(Vec::new());
    docx.build().pack(&mut cursor).unwrap();

    let document = Registry::default()
        .extract(
            source("note.docx", cursor.into_inner()),
            &ExtractOptions::default(),
        )
        .unwrap();

    let text = document
        .blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph { spans, .. } => spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            Block::Heading { text, .. } => text.clone(),
            _ => String::new(),
        })
        .collect::<String>();
    assert!(text.contains("Office heading"));
    assert!(text.contains("Body text."));
}
