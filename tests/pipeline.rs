use aitools::error::Error;
use aitools::extract::{ExtractOptions, Limits, Registry};
use aitools::model::{Block, FileKind, Source, SourceMeta};
use aitools::render::{RenderFormat, render};

fn source(name: &str, bytes: &[u8]) -> Source {
    Source {
        meta: SourceMeta {
            name: name.to_owned(),
            mime: "text/plain".to_owned(),
            size_bytes: bytes.len() as u64,
        },
        bytes: bytes.to_vec(),
    }
}

#[test]
fn markdown_becomes_blocks_and_markdown() {
    let source = source("note.md", b"# Title\n\nFirst paragraph.\n\nSecond line.\n");
    let document = Registry::default()
        .extract(source, &ExtractOptions::default())
        .expect("extraction should succeed");

    assert!(matches!(
        document.blocks.first(),
        Some(Block::Heading { level: 1, .. })
    ));
    assert_eq!(document.blocks.len(), 3);
    assert!(
        render(&document, RenderFormat::Markdown)
            .unwrap()
            .starts_with("# Title\n")
    );
}

#[test]
fn output_limit_is_enforced() {
    let options = ExtractOptions {
        limits: Limits {
            max_input_bytes: 1024,
            max_output_chars: 5,
            max_pages: 1,
        },
        ..ExtractOptions::default()
    };

    let error = Registry::default()
        .extract(source("long.txt", b"a long paragraph"), &options)
        .unwrap_err();

    assert!(matches!(error, Error::LimitExceeded(_)));
}

#[test]
fn unknown_format_is_rejected() {
    let error = Registry::default()
        .extract(
            source("blob.bin", &[0xff, 0xfe, 0x00]),
            &ExtractOptions::default(),
        )
        .unwrap_err();

    assert!(matches!(error, Error::Unsupported(FileKind::Unknown)));
}
