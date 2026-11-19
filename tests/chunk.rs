use aitools::chunk::{ChunkOptions, chunk};
use aitools::extract::{ExtractOptions, Registry};
use aitools::model::{Block, Loc, Source, SourceMeta};

fn source(name: &str, text: &str) -> Source {
    let bytes = text.as_bytes().to_vec();
    Source {
        meta: SourceMeta {
            name: name.to_owned(),
            mime: "text/plain".to_owned(),
            size_bytes: bytes.len() as u64,
        },
        bytes,
    }
}

#[test]
fn chunking_respects_headings_and_token_budget() {
    let document = Registry::default()
        .extract(
            source(
                "article.md",
                "# Intro\n\nA short paragraph.\n\n## Details\n\nMore text here.",
            ),
            &ExtractOptions::default(),
        )
        .unwrap();

    let chunks = chunk(&document, &ChunkOptions::default()).unwrap();
    assert!(!chunks.is_empty());
    assert!(chunks.iter().all(|chunk| chunk.tokens <= 800));
    assert!(chunks
        .iter()
        .all(|chunk| chunk.index == chunks.iter().position(|c| c.index == chunk.index).unwrap()));
    assert!(chunks.iter().any(|chunk| !chunk.headings.is_empty()));
}

#[test]
fn chunking_tracks_locations() {
    let mut document = aitools::model::Document::new(SourceMeta {
        name: "report.pdf".to_owned(),
        mime: "application/pdf".to_owned(),
        size_bytes: 0,
    });
    document.blocks.push(Block::Paragraph {
        spans: vec![aitools::model::Span::plain("Page one text.")],
        loc: Loc {
            page: Some(1),
            ..Loc::default()
        },
    });
    document.blocks.push(Block::PageBreak);
    document.blocks.push(Block::Paragraph {
        spans: vec![aitools::model::Span::plain("Page two text.")],
        loc: Loc {
            page: Some(2),
            ..Loc::default()
        },
    });

    let chunks = chunk(&document, &ChunkOptions::default()).unwrap();
    let all_pages: Vec<_> = chunks
        .iter()
        .flat_map(|chunk| chunk.locs.iter().filter_map(|loc| loc.page))
        .collect();
    assert!(all_pages.contains(&1));
    assert!(all_pages.contains(&2));
}
