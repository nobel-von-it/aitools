//! Token-aware chunking that respects document structure.

use serde::{Deserialize, Serialize};
use tiktoken_rs::cl100k_base_singleton;

use crate::error::Result;
use crate::model::{Block, Document, Loc};
use crate::render::to_markdown;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub index: usize,
    pub text: String,
    pub tokens: usize,
    /// Heading path at the start of the chunk.
    pub headings: Vec<String>,
    /// Source locations of the blocks included in the chunk.
    pub locs: Vec<Loc>,
}

#[derive(Debug, Clone)]
pub struct ChunkOptions {
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            max_tokens: 800,
            overlap_tokens: 80,
        }
    }
}

#[derive(Clone)]
struct Unit {
    text: String,
    tokens: usize,
    headings: Vec<String>,
    loc: Option<Loc>,
    forced_break: bool,
}

/// Split a document into chunks no larger than `max_tokens`, preferring
/// boundaries at headings, pages and sheets.
pub fn chunk(document: &Document, options: &ChunkOptions) -> Result<Vec<Chunk>> {
    let encoding = cl100k_base_singleton();
    let mut units = Vec::new();
    let mut headings: Vec<String> = Vec::new();

    for block in &document.blocks {
        let (forced_break, loc) = match block {
            Block::Heading { level, text, loc } => {
                let level = (*level as usize).clamp(1, 6);
                headings.truncate(level - 1);
                headings.push(text.clone());
                (true, Some(*loc))
            }
            Block::PageBreak => (true, None),
            Block::SheetBreak { name } => {
                headings.clear();
                headings.push(name.clone());
                (true, None)
            }
            Block::Paragraph { loc, .. } | Block::Table { loc, .. } | Block::Image { loc, .. } => {
                (false, Some(*loc))
            }
        };

        let text = render_block(block).trim().to_owned();
        if text.is_empty() {
            continue;
        }
        units.push(Unit {
            tokens: encoding.encode_with_special_tokens(&text).len(),
            text,
            headings: headings.clone(),
            loc,
            forced_break,
        });
    }

    let mut chunks = Vec::new();
    let mut current: Vec<Unit> = Vec::new();
    let mut current_tokens = 0;

    for unit in units {
        let would_exceed = current_tokens + unit.tokens > options.max_tokens && !current.is_empty();
        if would_exceed || (unit.forced_break && !current.is_empty()) {
            let (chunk, mut overlap) = assemble(
                std::mem::take(&mut current),
                chunks.len(),
                encoding,
                options.overlap_tokens,
            );
            if unit.forced_break {
                overlap.clear();
            }
            chunks.push(chunk);
            current = overlap;
            current_tokens = current.iter().map(|unit| unit.tokens).sum();
        }
        current_tokens += unit.tokens;
        current.push(unit);
    }

    if !current.is_empty() {
        let (chunk, _) = assemble(current, chunks.len(), encoding, options.overlap_tokens);
        chunks.push(chunk);
    }

    Ok(chunks)
}

fn render_block(block: &Block) -> String {
    let mut document = Document::new(crate::model::SourceMeta {
        name: String::new(),
        mime: String::new(),
        size_bytes: 0,
    });
    document.blocks.push(block.clone());
    to_markdown(&document)
}

fn assemble(
    units: Vec<Unit>,
    index: usize,
    encoding: &tiktoken_rs::CoreBPE,
    overlap_tokens: usize,
) -> (Chunk, Vec<Unit>) {
    let headings = units
        .first()
        .map_or(Vec::new(), |unit| unit.headings.clone());
    let mut text = String::new();
    if !headings.is_empty() {
        text.push_str(&format!("> context: {}\n\n", headings.join(" > ")));
    }
    for unit in &units {
        text.push_str(&unit.text);
        text.push_str("\n\n");
    }
    let text = text.trim().to_owned();

    let mut overlap = Vec::new();
    let mut used_tokens = 0;
    for unit in units.iter().rev() {
        if used_tokens + unit.tokens > overlap_tokens {
            break;
        }
        used_tokens += unit.tokens;
        overlap.push((*unit).clone());
    }
    overlap.reverse();

    let chunk = Chunk {
        index,
        tokens: encoding.encode_with_special_tokens(&text).len(),
        text,
        headings,
        locs: units.into_iter().filter_map(|unit| unit.loc).collect(),
    };

    (chunk, overlap)
}
