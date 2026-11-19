use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{Document, FileKind, Source};
use crate::ocr::{OcrConfig, VisionOcr};
use crate::text::PlainTextExtractor;

#[cfg(feature = "image")]
use crate::images::ImageExtractor;

#[cfg(feature = "office")]
use crate::office::{DocxExtractor, XlsxExtractor};

#[cfg(feature = "pdf")]
use crate::pdf::PdfExtractor;

/// Resource bounds applied to every extraction call.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_output_chars: usize,
    pub max_pages: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_output_chars: 2_000_000,
            max_pages: 200,
        }
    }
}

/// OCR backend chosen per call; `Auto` picks the best available one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OcrMode {
    #[default]
    Off,
    Auto,
    Ocrs,
    Tesseract,
    Vision,
}

#[derive(Clone, Default)]
pub struct ExtractOptions {
    pub limits: Limits,
    pub ocr: OcrMode,
    pub ocr_config: OcrConfig,
    pub include_images: bool,
    /// Optional vision-language backend for `OcrMode::Vision` and `Auto`.
    pub vision: Option<Arc<dyn VisionOcr>>,
}

/// A format-specific parser that fills a [`Document`].
pub trait Extractor: Send + Sync {
    fn kinds(&self) -> &'static [FileKind];

    fn extract(&self, source: &Source, opts: &ExtractOptions, out: &mut Document) -> Result<()>;
}

/// Format detector and extractor dispatcher.
pub struct Registry {
    extractors: Vec<Box<dyn Extractor>>,
}

impl Default for Registry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(PlainTextExtractor);
        #[cfg(feature = "pdf")]
        registry.register(PdfExtractor);
        #[cfg(feature = "image")]
        registry.register(ImageExtractor);
        #[cfg(feature = "office")]
        {
            registry.register(DocxExtractor);
            registry.register(XlsxExtractor);
        }
        registry
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            extractors: Vec::new(),
        }
    }

    pub fn register(&mut self, extractor: impl Extractor + 'static) {
        self.extractors.push(Box::new(extractor));
    }

    pub fn supported(&self) -> Vec<FileKind> {
        let mut kinds: Vec<FileKind> = self
            .extractors
            .iter()
            .flat_map(|extractor| extractor.kinds())
            .copied()
            .collect();
        kinds.sort_by_key(|kind| *kind as usize);
        kinds.dedup();
        kinds
    }

    pub fn extract(&self, source: Source, opts: &ExtractOptions) -> Result<Document> {
        if source.bytes.len() > opts.limits.max_input_bytes {
            return Err(Error::LimitExceeded(format!(
                "input is {} bytes, limit is {}",
                source.bytes.len(),
                opts.limits.max_input_bytes
            )));
        }

        let kind = source.kind();
        let extractor = self
            .extractors
            .iter()
            .find(|extractor| extractor.kinds().contains(&kind))
            .ok_or(Error::Unsupported(kind))?;

        let mut document = Document::new(source.meta.clone());
        extractor.extract(&source, opts, &mut document)?;

        let chars = document.char_count();
        if chars > opts.limits.max_output_chars {
            return Err(Error::LimitExceeded(format!(
                "output is {chars} chars, limit is {}",
                opts.limits.max_output_chars
            )));
        }

        Ok(document)
    }
}
