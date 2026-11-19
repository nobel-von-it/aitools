use std::process::ExitCode;

use aitools::chunk::{ChunkOptions, chunk};
use aitools::extract::{ExtractOptions, Limits, OcrMode, Registry};
use aitools::model::Source;
use aitools::render::{RenderFormat, render};
use aitools::{OcrConfig, VisionOcr};

fn print_usage() {
    eprintln!(
        "Usage: aitools <path> [options]\n\
         \n\
         Options:\n\
         \x20 --format md|json       Output format (default: md)\n\
         \x20 --chunk                Emit token-aware chunks as JSON\n\
         \x20 --ocr off|auto|ocrs|tesseract|vision\n\
         \x20                         OCR backend for images (default: off)\n\
         \x20 --include-images       Include raw image bytes in JSON output\n\
         \x20 --tesseract-lang LANG  Language codes for Tesseract (default: eng)\n\
         \x20 --max-tokens N         Chunk size in tokens (default: 800)\n\
         \x20 --overlap-tokens N     Chunk overlap in tokens (default: 80)\n\
         \x20 --max-output-chars N   Extraction output limit (default: 2000000)\n\
         \x20 --max-pages N          Maximum pages/sheets to process (default: 200)\n\
         \x20 --help                 Show this help"
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return ExitCode::from(2);
    }

    let path = &args[0];
    let mut format = RenderFormat::Markdown;
    let mut emit_chunks = false;
    let mut ocr = OcrMode::Off;
    let mut include_images = false;
    let mut tesseract_lang = "eng".to_owned();
    let mut chunk_options = ChunkOptions::default();
    let mut limits = Limits::default();

    let mut index = 1;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args.get(index + 1).map(String::as_str);
        match flag {
            "--format" => match value {
                Some("json") => format = RenderFormat::Json,
                Some("md") | Some("markdown") => format = RenderFormat::Markdown,
                _ => {
                    eprintln!("Unknown format. Use 'md' or 'json'.");
                    return ExitCode::from(2);
                }
            },
            "--ocr" => match value {
                Some("off") => ocr = OcrMode::Off,
                Some("auto") => ocr = OcrMode::Auto,
                Some("ocrs") => ocr = OcrMode::Ocrs,
                Some("tesseract") => ocr = OcrMode::Tesseract,
                Some("vision") => ocr = OcrMode::Vision,
                _ => {
                    eprintln!("Unknown OCR mode. Use off, auto, ocrs, tesseract, or vision.");
                    return ExitCode::from(2);
                }
            },
            "--tesseract-lang" => {
                if let Some(value) = value {
                    tesseract_lang = value.to_owned();
                }
            }
            "--include-images" => include_images = true,
            "--chunk" => emit_chunks = true,
            "--max-tokens" => {
                if let Some(value) = value.and_then(|value| value.parse().ok()) {
                    chunk_options.max_tokens = value;
                }
            }
            "--overlap-tokens" => {
                if let Some(value) = value.and_then(|value| value.parse().ok()) {
                    chunk_options.overlap_tokens = value;
                }
            }
            "--max-output-chars" => {
                if let Some(value) = value.and_then(|value| value.parse().ok()) {
                    limits.max_output_chars = value;
                }
            }
            "--max-pages" => {
                if let Some(value) = value.and_then(|value| value.parse().ok()) {
                    limits.max_pages = value;
                }
            }
            other => {
                eprintln!("Unknown option: {other}");
                print_usage();
                return ExitCode::from(2);
            }
        }
        // Consume the value of flags that take one.
        if matches!(
            flag,
            "--format"
                | "--ocr"
                | "--tesseract-lang"
                | "--max-tokens"
                | "--overlap-tokens"
                | "--max-output-chars"
                | "--max-pages"
        ) {
            index += 1;
        }
        index += 1;
    }

    let source = match Source::from_path(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("Cannot read input: {error}");
            return ExitCode::FAILURE;
        }
    };

    let options = ExtractOptions {
        limits,
        ocr,
        include_images,
        ocr_config: OcrConfig {
            tesseract_lang,
            ..OcrConfig::default()
        },
        vision: None::<std::sync::Arc<dyn VisionOcr>>,
    };

    let document = match Registry::default().extract(source, &options) {
        Ok(document) => document,
        Err(error) => {
            eprintln!("Extraction failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    if emit_chunks {
        match chunk(&document, &chunk_options)
            .and_then(|chunks| serde_json::to_string_pretty(&chunks).map_err(Into::into))
        {
            Ok(output) => {
                println!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("Chunking failed: {error}");
                ExitCode::FAILURE
            }
        }
    } else {
        match render(&document, format) {
            Ok(output) => {
                println!("{output}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("Rendering failed: {error}");
                ExitCode::FAILURE
            }
        }
    }
}
