//! Image backend: metadata, dimensions, optional OCR text.

use std::io::Cursor;

use crate::error::Result;
use crate::extract::{ExtractOptions, Extractor, OcrMode};
use crate::model::{Block, Document, FileKind, Loc, Source, Span, Warning};
use crate::ocr::recognize;

pub struct ImageExtractor;

impl Extractor for ImageExtractor {
    fn kinds(&self) -> &'static [FileKind] {
        &[FileKind::Image]
    }

    fn extract(&self, source: &Source, opts: &ExtractOptions, out: &mut Document) -> Result<()> {
        let format = image::guess_format(&source.bytes).ok();
        let dimensions = image::load_from_memory(&source.bytes)
            .ok()
            .map(|image| (image.width(), image.height()));
        let mime = format
            .map(|format| format.to_mime_type().to_owned())
            .unwrap_or_else(|| source.meta.mime.clone());

        let mut alt = format!("image {}", source.meta.name);
        if let Some((width, height)) = dimensions {
            alt.push_str(&format!(" {width}x{height}"));
        }
        for detail in exif_details(&source.bytes) {
            alt.push_str("; ");
            alt.push_str(&detail);
        }

        out.blocks.push(Block::Image {
            data: if opts.include_images {
                Some(source.bytes.clone())
            } else {
                None
            },
            mime: mime.clone(),
            alt: Some(alt),
            loc: Loc::default(),
        });

        if format.is_none() {
            out.warnings.push(Warning::new(
                "unknown_image_format",
                "image could not be decoded; only raw metadata is available",
            ));
        }

        if opts.ocr != OcrMode::Off {
            match recognize(
                &source.bytes,
                &mime,
                opts.ocr,
                &opts.ocr_config,
                opts.vision.as_deref(),
            ) {
                Ok((text, backend)) => {
                    out.warnings.push(Warning::new(
                        "ocr_backend",
                        format!("text recognized with {backend}"),
                    ));
                    let text = text.trim();
                    if !text.is_empty() {
                        out.blocks.push(Block::Paragraph {
                            spans: vec![Span::plain(text)],
                            loc: Loc::default(),
                        });
                    }
                }
                Err(warning) => out.warnings.push(warning),
            }
        }

        Ok(())
    }
}

fn exif_details(bytes: &[u8]) -> Vec<String> {
    let reader = exif::Reader::new();
    let Ok(metadata) = reader.read_from_container(&mut Cursor::new(bytes)) else {
        return Vec::new();
    };

    [
        exif::Tag::Make,
        exif::Tag::Model,
        exif::Tag::DateTime,
        exif::Tag::Orientation,
    ]
    .into_iter()
    .filter_map(|tag| metadata.get_field(tag, exif::In::PRIMARY))
    .map(|field| field.display_value().with_unit(&metadata).to_string())
    .filter(|value| !value.is_empty())
    .collect()
}
