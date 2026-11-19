use crate::error::Result;
use crate::extract::{ExtractOptions, Extractor};
use crate::model::{Block, Document, FileKind, Loc, Source, Span, Warning};

/// Vertical-slice extractor for plain text and Markdown.
pub struct PlainTextExtractor;

impl Extractor for PlainTextExtractor {
    fn kinds(&self) -> &'static [FileKind] {
        &[FileKind::Text, FileKind::Markdown]
    }

    fn extract(&self, source: &Source, _opts: &ExtractOptions, out: &mut Document) -> Result<()> {
        let text = String::from_utf8_lossy(&source.bytes);
        let markdown = source.kind() == FileKind::Markdown;
        let mut paragraph = String::new();
        let mut loc = Loc::default();

        for line in text.lines() {
            let trimmed = line.trim();
            let heading = markdown && trimmed.starts_with('#') && trimmed.contains(' ');

            if heading || trimmed.is_empty() {
                flush_paragraph(&mut paragraph, &mut out.blocks, &mut loc);
            }

            if heading {
                let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
                let level = (hashes as u8).clamp(1, 6);
                out.blocks.push(Block::Heading {
                    level,
                    text: trimmed[hashes..].trim().to_owned(),
                    loc,
                });
            } else if !trimmed.is_empty() {
                if !paragraph.is_empty() {
                    paragraph.push(' ');
                }
                paragraph.push_str(trimmed);
            }
        }

        flush_paragraph(&mut paragraph, &mut out.blocks, &mut loc);

        if out.blocks.is_empty() {
            out.warnings
                .push(Warning::new("empty_document", "no text content found"));
        }

        Ok(())
    }
}

fn flush_paragraph(paragraph: &mut String, blocks: &mut Vec<Block>, loc: &mut Loc) {
    if paragraph.is_empty() {
        return;
    }
    blocks.push(Block::Paragraph {
        spans: vec![Span::plain(std::mem::take(paragraph))],
        loc: *loc,
    });
}
