#![cfg(feature = "image")]

use std::sync::Arc;

use aitools::extract::{ExtractOptions, OcrMode, Registry};
use aitools::model::{Block, Source, SourceMeta};
use aitools::{Result, VisionOcr};

struct FakeVision;

impl VisionOcr for FakeVision {
    fn recognize(&self, image: &[u8], mime: &str) -> Result<String> {
        assert_eq!(mime, "image/png");
        Ok(format!("vision text ({image} bytes)", image = image.len()))
    }
}

fn png_source() -> Source {
    let image = image::DynamicImage::new_rgb8(2, 2);
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();

    Source {
        meta: SourceMeta {
            name: "picture.png".to_owned(),
            mime: "image/png".to_owned(),
            size_bytes: bytes.len() as u64,
        },
        bytes,
    }
}

#[test]
fn image_becomes_block_with_dimensions() {
    let document = Registry::default()
        .extract(png_source(), &ExtractOptions::default())
        .unwrap();

    let image = document
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::Image { mime, alt, .. } => Some((mime.as_str(), alt.as_deref().unwrap_or(""))),
            _ => None,
        })
        .unwrap();
    assert_eq!(image.0, "image/png");
    assert!(image.1.contains("2x2"));
}

#[cfg(feature = "ocr-vision")]
#[test]
fn vision_backend_adds_ocr_text() {
    let options = ExtractOptions {
        ocr: OcrMode::Vision,
        vision: Some(Arc::new(FakeVision)),
        ..ExtractOptions::default()
    };

    let document = Registry::default().extract(png_source(), &options).unwrap();
    let text = document
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph { spans, .. } => Some(
                spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect::<String>();

    assert!(text.contains("vision text"));
    assert!(
        document
            .warnings
            .iter()
            .any(|warning| warning.code == "ocr_backend")
    );
}
