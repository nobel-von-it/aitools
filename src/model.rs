use std::path::Path;

use serde::{Deserialize, Serialize};

/// Format categories understood by the extractor registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Text,
    Markdown,
    Pdf,
    Docx,
    Xlsx,
    Image,
    Unknown,
}

impl FileKind {
    /// Detect by magic bytes first, extension second.
    pub fn detect(bytes: &[u8], name: Option<&str>) -> Self {
        infer::get(bytes)
            .and_then(|kind| mime_to_kind(kind.mime_type()))
            .or_else(|| {
                name.and_then(|n| Path::new(n).extension())
                    .and_then(|ext| ext.to_str())
                    .and_then(ext_to_kind)
            })
            .unwrap_or(FileKind::Unknown)
    }
}

fn mime_to_kind(mime: &str) -> Option<FileKind> {
    match mime {
        "application/pdf" => Some(FileKind::Pdf),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            Some(FileKind::Docx)
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some(FileKind::Xlsx),
        mime if mime.starts_with("image/") => Some(FileKind::Image),
        mime if mime.starts_with("text/") => Some(FileKind::Text),
        _ => None,
    }
}

fn ext_to_kind(ext: &str) -> Option<FileKind> {
    match ext.to_ascii_lowercase().as_str() {
        "txt" | "log" | "csv" | "json" | "xml" | "yaml" | "yml" => Some(FileKind::Text),
        "md" | "markdown" => Some(FileKind::Markdown),
        "pdf" => Some(FileKind::Pdf),
        "docx" => Some(FileKind::Docx),
        "xlsx" | "xlsm" | "xls" | "ods" => Some(FileKind::Xlsx),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "tiff" | "svg" => Some(FileKind::Image),
        _ => None,
    }
}

/// Descriptive metadata about the input file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMeta {
    pub name: String,
    pub mime: String,
    pub size_bytes: u64,
}

/// Raw input with its metadata.
#[derive(Debug, Clone)]
pub struct Source {
    pub meta: SourceMeta,
    pub bytes: Vec<u8>,
}

impl Source {
    pub fn from_path(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unnamed")
            .to_owned();
        let mime = infer::get(&bytes)
            .map(|kind| kind.mime_type().to_owned())
            .unwrap_or_else(|| "application/octet-stream".to_owned());

        Ok(Self {
            meta: SourceMeta {
                name,
                mime,
                size_bytes: bytes.len() as u64,
            },
            bytes,
        })
    }

    pub fn kind(&self) -> FileKind {
        FileKind::detect(&self.bytes, Some(&self.meta.name))
    }
}

/// A non-fatal issue reported alongside extracted content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

impl Warning {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Format-independent extraction result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub source: SourceMeta,
    pub blocks: Vec<Block>,
    pub warnings: Vec<Warning>,
}

impl Document {
    pub fn new(source: SourceMeta) -> Self {
        Self {
            source,
            blocks: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn char_count(&self) -> usize {
        self.blocks.iter().map(Block::char_count).sum()
    }
}

/// A single semantic piece of a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Block {
    Heading {
        level: u8,
        text: String,
        loc: Loc,
    },
    Paragraph {
        spans: Vec<Span>,
        loc: Loc,
    },
    Table {
        rows: Vec<Vec<String>>,
        loc: Loc,
    },
    Image {
        data: Option<Vec<u8>>,
        mime: String,
        alt: Option<String>,
        loc: Loc,
    },
    PageBreak,
    SheetBreak {
        name: String,
    },
}

impl Block {
    pub fn char_count(&self) -> usize {
        match self {
            Self::Heading { text, .. } => text.chars().count(),
            Self::Paragraph { spans, .. } => spans.iter().map(|s| s.text.chars().count()).sum(),
            Self::Table { rows, .. } => rows
                .iter()
                .flat_map(|row| row.iter())
                .map(|cell| cell.chars().count())
                .sum(),
            Self::Image { alt, .. } => alt.as_ref().map_or(0, |alt| alt.chars().count()),
            Self::PageBreak => 0,
            Self::SheetBreak { name } => name.chars().count(),
        }
    }
}

/// Inline text fragment with optional styling.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: Style::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

/// Source location of a block, when the format provides one.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Loc {
    pub page: Option<u32>,
    pub sheet: Option<u32>,
    pub section: Option<u32>,
    pub bbox: Option<Rect>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
