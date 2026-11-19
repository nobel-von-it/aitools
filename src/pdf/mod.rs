//! PDF backend: `pdf_oxide` for positioned spans, `pdf-extract` as a fallback.

use pdf_oxide::layout::TextSpan;
use pdf_oxide::{PdfDocument, ReadingOrder};

use crate::error::Result;
use crate::extract::{ExtractOptions, Extractor};
use crate::model::{Block, Document, FileKind, Loc, Rect, Source, Span, Style, Warning};

const MAX_IMAGES_PER_PAGE: usize = 16;

pub struct PdfExtractor;

impl Extractor for PdfExtractor {
    fn kinds(&self) -> &'static [FileKind] {
        &[FileKind::Pdf]
    }

    fn extract(&self, source: &Source, opts: &ExtractOptions, out: &mut Document) -> Result<()> {
        let document = match PdfDocument::from_bytes(source.bytes.clone()) {
            Ok(document) => document,
            Err(error) => {
                fallback_text(source, out, &error.to_string());
                return Ok(());
            }
        };

        if document.is_encrypted() {
            out.warnings.push(Warning::new(
                "encrypted",
                "PDF is encrypted; only content readable without a password was extracted",
            ));
        }

        let page_count = document.page_count().unwrap_or(0);
        if page_count == 0 {
            out.warnings.push(Warning::new(
                "no_pages",
                "primary parser found no pages; trying fallback",
            ));
            fallback_text(source, out, "zero pages");
            return Ok(());
        }

        if page_count > opts.limits.max_pages {
            out.warnings.push(Warning::new(
                "page_limit",
                format!(
                    "document has {page_count} pages, only first {} were extracted",
                    opts.limits.max_pages
                ),
            ));
        }

        let pages = page_count.min(opts.limits.max_pages);
        for index in 0..pages {
            if index > 0 {
                out.blocks.push(Block::PageBreak);
            }

            let spans = document
                .extract_spans_with_reading_order(index, ReadingOrder::ColumnAware)
                .or_else(|_| document.extract_spans(index))
                .unwrap_or_default();

            if spans.is_empty() {
                out.warnings.push(Warning::new(
                    "no_text_layer",
                    format!("page {} has no extractable text layer", index + 1),
                ));
            }

            out.blocks.extend(spans_to_blocks(spans, index as u32 + 1));

            if opts.include_images {
                extract_images(&document, index, index as u32 + 1, out);
            }
        }

        out.warnings.push(Warning::new(
            "pdf_strategy",
            if out.blocks.is_empty() {
                "pdf-extract"
            } else {
                "pdf_oxide"
            },
        ));

        if out.blocks.is_empty() {
            fallback_text(source, out, "no blocks produced by primary parser");
        }

        Ok(())
    }
}

fn fallback_text(source: &Source, out: &mut Document, reason: &str) {
    out.warnings.push(Warning::new(
        "pdf_fallback",
        format!("primary parser failed ({reason}); using pdf-extract"),
    ));

    match pdf_extract::extract_text_from_mem_by_pages(&source.bytes) {
        Ok(pages) => {
            for (index, text) in pages.iter().enumerate() {
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                out.blocks.push(Block::Paragraph {
                    spans: vec![Span::plain(text)],
                    loc: Loc {
                        page: Some(index as u32 + 1),
                        sheet: None,
                        section: None,
                        bbox: None,
                    },
                });
            }
        }
        Err(error) => out.warnings.push(Warning::new(
            "fallback_failed",
            format!("pdf-extract could not parse the file: {error}"),
        )),
    }
}

fn extract_images(document: &PdfDocument, page_index: usize, page: u32, out: &mut Document) {
    let images = match document.extract_images(page_index) {
        Ok(images) => images,
        Err(error) => {
            out.warnings.push(Warning::new(
                "image_extraction_failed",
                format!("page {page}: {error}"),
            ));
            return;
        }
    };

    if images.len() > MAX_IMAGES_PER_PAGE {
        out.warnings.push(Warning::new(
            "image_limit",
            format!(
                "page {page} has {} images, only {MAX_IMAGES_PER_PAGE} were included",
                images.len()
            ),
        ));
    }

    for image in images.iter().take(MAX_IMAGES_PER_PAGE) {
        match image.to_png_bytes() {
            Ok(png) => out.blocks.push(Block::Image {
                data: Some(png),
                mime: "image/png".to_owned(),
                alt: Some(format!("image {}x{}", image.width(), image.height())),
                loc: Loc {
                    page: Some(page),
                    sheet: None,
                    section: None,
                    bbox: image.bbox().map(|rect| Rect {
                        x: rect.x,
                        y: rect.y,
                        width: rect.width,
                        height: rect.height,
                    }),
                },
            }),
            Err(error) => out.warnings.push(Warning::new(
                "image_decode_failed",
                format!("page {page}: {error}"),
            )),
        }
    }
}

struct Line {
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
    font_size: f32,
    bold: bool,
    heading: Option<u8>,
    spans: Vec<Span>,
    text: String,
}

fn spans_to_blocks(spans: Vec<TextSpan>, page: u32) -> Vec<Block> {
    let mut sizes: Vec<f32> = spans.iter().map(|span| span.font_size.max(1.0)).collect();
    sizes.sort_by(|left, right| left.total_cmp(right));
    let median_size = sizes
        .get(sizes.len().saturating_sub(1) / 2)
        .copied()
        .unwrap_or(10.0);

    let mut lines: Vec<Line> = Vec::new();
    for span in spans {
        let text = span.text.trim();
        if text.is_empty() {
            continue;
        }

        let rect = span.bbox;
        let font_size = span.font_size.max(1.0);
        let style = Style {
            bold: span.font_weight.is_bold(),
            italic: span.is_italic,
            code: span.is_monospace,
        };

        if let Some(line) = lines.last_mut() {
            let same_line = (line.top - rect.y).abs() <= line.font_size * 0.6
                && rect.x >= line.right - font_size;
            if same_line {
                if rect.x > line.right + font_size * 0.2 {
                    line.spans.push(Span::plain(" "));
                    line.text.push(' ');
                }
                line.spans.push(Span {
                    text: text.to_owned(),
                    style,
                });
                line.text.push_str(text);
                line.left = line.left.min(rect.x);
                line.right = line.right.max(rect.x + rect.width);
                line.top = line.top.min(rect.y);
                line.bottom = line.bottom.max(rect.y + rect.height);
                line.font_size = line.font_size.max(font_size);
                line.bold |= style.bold;
                continue;
            }
        }

        lines.push(Line {
            top: rect.y,
            bottom: rect.y + rect.height,
            left: rect.x,
            right: rect.x + rect.width,
            font_size,
            bold: style.bold,
            heading: None,
            spans: vec![Span {
                text: text.to_owned(),
                style,
            }],
            text: text.to_owned(),
        });
    }

    for line in &mut lines {
        let ratio = line.font_size / median_size;
        let short = line.text.chars().count() <= 160;
        if short && (ratio >= 1.2 || (line.bold && ratio >= 1.05)) {
            line.heading = Some(if ratio >= 1.6 {
                1
            } else if ratio >= 1.3 {
                2
            } else {
                3
            });
        }
    }

    let mut blocks = Vec::new();
    let mut pending: Vec<Line> = Vec::new();
    for line in lines {
        let should_flush = pending.last().is_some_and(|previous| {
            line.heading.is_some()
                || previous.heading.is_some()
                || line.top - previous.bottom > previous.font_size * 0.8
        });
        if should_flush {
            flush_lines(&mut pending, &mut blocks, page);
        }
        pending.push(line);
    }
    flush_lines(&mut pending, &mut blocks, page);

    blocks
}

fn flush_lines(lines: &mut Vec<Line>, blocks: &mut Vec<Block>, page: u32) {
    if lines.is_empty() {
        return;
    }

    let left = lines.iter().map(|line| line.left).fold(f32::MAX, f32::min);
    let top = lines.iter().map(|line| line.top).fold(f32::MAX, f32::min);
    let right = lines.iter().map(|line| line.right).fold(f32::MIN, f32::max);
    let bottom = lines
        .iter()
        .map(|line| line.bottom)
        .fold(f32::MIN, f32::max);
    let loc = Loc {
        page: Some(page),
        sheet: None,
        section: None,
        bbox: Some(Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }),
    };

    if lines.len() == 1 && lines[0].heading.is_some() {
        blocks.push(Block::Heading {
            level: lines[0].heading.unwrap(),
            text: lines[0].text.clone(),
            loc,
        });
    } else {
        let mut spans = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                spans.push(Span::plain(" "));
            }
            spans.extend(line.spans.iter().cloned());
        }
        blocks.push(Block::Paragraph { spans, loc });
    }

    lines.clear();
}
