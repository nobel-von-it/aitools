#![cfg(feature = "pdf")]

use aitools::extract::{ExtractOptions, Registry};
use aitools::model::{Block, Source, SourceMeta};
use aitools::render::{RenderFormat, render};

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Minimal two-page PDF with a large heading and body text on each page.
fn sample_pdf() -> Vec<u8> {
    let pages = [
        ("Chapter One", "Hello from page one."),
        ("Chapter Two", "Hello from page two."),
    ];

    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            (0..pages.len())
                .map(|index| format!("{} 0 R", 4 + index * 2))
                .collect::<Vec<_>>()
                .join(" "),
            pages.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
    ];

    for (index, (title, body)) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + index * 2
        ));

        let stream = format!(
            "BT /F1 24 Tf 72 700 Td ({}) Tj ET\nBT /F1 11 Tf 72 650 Td ({}) Tj ET\n",
            escape(title),
            escape(body)
        );
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}endstream",
            stream.len(),
            stream
        ));
    }

    let mut output = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
    }

    let xref_offset = output.len();
    output.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    output.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );

    output
}

#[test]
fn extracts_pdf_pages_with_headings_and_locations() {
    let bytes = sample_pdf();
    let source = Source {
        meta: SourceMeta {
            name: "sample.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size_bytes: bytes.len() as u64,
        },
        bytes,
    };

    let document = Registry::default()
        .extract(source, &ExtractOptions::default())
        .expect("PDF should be parsed");

    let headings: Vec<_> = document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Heading { text, loc, .. } => Some((text.as_str(), loc.page)),
            _ => None,
        })
        .collect();
    assert_eq!(
        headings,
        vec![("Chapter One", Some(1)), ("Chapter Two", Some(2))]
    );
    assert!(
        document
            .blocks
            .iter()
            .any(|block| matches!(block, Block::PageBreak))
    );
    assert!(
        document
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Paragraph { loc, .. } if loc.bbox.is_some()))
    );

    let markdown = render(&document, RenderFormat::Markdown).unwrap();
    assert!(markdown.contains("# Chapter One"));
    assert!(markdown.contains("Hello from page two."));
}
