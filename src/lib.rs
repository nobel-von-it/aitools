//! Universal document extraction toolkit for AI agents.
//!
//! The crate is built around a format-independent intermediate representation
//! ([`model::Document`]) and a registry of pluggable [`extract::Extractor`]s.
//! Renderers turn that representation into Markdown or JSON for an agent's
//! context, while future chunks preserve source locations.

pub mod chunk;
pub mod error;
pub mod extract;
pub mod model;
pub mod render;

mod text;

#[cfg(feature = "image")]
pub mod images;

pub mod ocr;

#[cfg(feature = "office")]
pub mod office;

#[cfg(feature = "pdf")]
pub mod pdf;

pub use chunk::{Chunk, ChunkOptions};
pub use error::{Error, Result};
pub use extract::{ExtractOptions, Extractor, Limits, OcrMode, Registry};
pub use model::{Block, Document, FileKind, Loc, Source, SourceMeta, Span, Style, Warning};
pub use ocr::{OcrConfig, VisionOcr};
pub use render::{RenderFormat, render};

#[cfg(feature = "pdf")]
pub use pdf::PdfExtractor;

#[cfg(feature = "office")]
pub use office::{DocxExtractor, XlsxExtractor};

#[cfg(feature = "image")]
pub use images::ImageExtractor;
