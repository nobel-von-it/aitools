use crate::error::Result;
use crate::model::{Block, Document, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    Markdown,
    Json,
}

/// Render a [`Document`] into a format suitable for an agent's context.
pub fn render(document: &Document, format: RenderFormat) -> Result<String> {
    match format {
        RenderFormat::Markdown => Ok(to_markdown(document)),
        RenderFormat::Json => Ok(serde_json::to_string_pretty(document)?),
    }
}

pub fn to_markdown(document: &Document) -> String {
    let mut output = String::new();

    for block in &document.blocks {
        match block {
            Block::Heading { level, text, .. } => {
                let hashes = "#".repeat((*level).clamp(1, 6) as usize);
                output.push_str(&hashes);
                output.push(' ');
                output.push_str(text);
            }
            Block::Paragraph { spans, .. } => {
                for span in spans {
                    render_span(&mut output, span);
                }
            }
            Block::Table { rows, .. } => {
                let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
                if let Some(header) = rows.first() {
                    output.push_str("| ");
                    output.push_str(&render_row(header, columns));
                    output.push_str(" |\n| ");
                    output.push_str(&vec!["---"; columns].join(" | "));
                    output.push('|');
                }
                for row in rows.iter().skip(1) {
                    output.push_str("\n| ");
                    output.push_str(&render_row(row, columns));
                    output.push('|');
                }
            }
            Block::Image { alt, mime, .. } => {
                output.push_str("![");
                if let Some(alt) = alt {
                    output.push_str(&alt.replace(['[', ']'], ""));
                }
                output.push_str("](");
                output.push_str(mime);
                output.push(')');
            }
            Block::PageBreak => output.push_str("---"),
            Block::SheetBreak { name } => {
                output.push_str("## Sheet: ");
                output.push_str(name);
            }
        }
        output.push_str("\n\n");
    }

    for warning in &document.warnings {
        output.push_str(&format!("> [{}] {}\n", warning.code, warning.message));
    }

    output
}

fn render_span(output: &mut String, span: &Span) {
    if span.style.code {
        output.push('`');
    }
    if span.style.bold {
        output.push_str("**");
    }
    if span.style.italic {
        output.push('*');
    }
    output.push_str(&span.text);
    if span.style.italic {
        output.push('*');
    }
    if span.style.bold {
        output.push_str("**");
    }
    if span.style.code {
        output.push('`');
    }
}

fn render_row(row: &[String], columns: usize) -> String {
    (0..columns)
        .map(|index| {
            row.get(index)
                .map(|cell| cell.replace('|', "\\|"))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" | ")
}
