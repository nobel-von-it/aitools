//! Pluggable OCR backends selected at runtime by [`crate::extract::OcrMode`].

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::error::Result;
use crate::extract::OcrMode;
use crate::model::Warning;

/// Configuration for the OCR backends.
#[derive(Debug, Clone)]
pub struct OcrConfig {
    /// Language codes passed to Tesseract (for example `rus+eng`).
    pub tesseract_lang: String,
    /// ONNX text-detection model for `ocrs`.
    pub ocrs_detection_model: Option<PathBuf>,
    /// ONNX text-recognition model for `ocrs`.
    pub ocrs_recognition_model: Option<PathBuf>,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            tesseract_lang: "eng".to_owned(),
            ocrs_detection_model: std::env::var_os("AITOOLS_OCRS_DETECTION_MODEL")
                .map(PathBuf::from),
            ocrs_recognition_model: std::env::var_os("AITOOLS_OCRS_RECOGNITION_MODEL")
                .map(PathBuf::from),
        }
    }
}

/// Hook for a vision-language model supplied by the agent runtime.
pub trait VisionOcr: Send + Sync {
    fn recognize(&self, image: &[u8], mime: &str) -> Result<String>;
}

type OcrOutcome = std::result::Result<(String, &'static str), Warning>;

/// Run the requested OCR backend. On failure returns a warning rather than an error:
/// missing OCR must not discard the rest of the document.
pub fn recognize(
    image: &[u8],
    mime: &str,
    mode: OcrMode,
    config: &OcrConfig,
    vision: Option<&dyn VisionOcr>,
) -> OcrOutcome {
    match mode {
        OcrMode::Off => Err(Warning::new(
            "ocr_disabled",
            "OCR is disabled for this call",
        )),
        OcrMode::Ocrs => run_ocrs(image, config),
        OcrMode::Tesseract => run_tesseract(image, config),
        OcrMode::Vision => run_vision(image, mime, vision),
        OcrMode::Auto => {
            #[cfg(feature = "ocr-ocrs")]
            if let Ok(result) = run_ocrs(image, config) {
                return Ok(result);
            }
            if let Ok(result) = run_tesseract(image, config) {
                return Ok(result);
            }
            if let Ok(result) = run_vision(image, mime, vision) {
                return Ok(result);
            }
            Err(Warning::new(
                "no_ocr_backend",
                "no OCR backend available: install tesseract, configure ocrs models, or inject a vision backend",
            ))
        }
    }
}

fn run_vision(image: &[u8], mime: &str, vision: Option<&dyn VisionOcr>) -> OcrOutcome {
    let vision = vision.ok_or_else(|| {
        Warning::new(
            "vision_backend_missing",
            "OcrMode::Vision requires a VisionOcr implementation in ExtractOptions",
        )
    })?;

    match vision.recognize(image, mime) {
        Ok(text) if !text.trim().is_empty() => Ok((text, "vision")),
        Ok(_) => Err(Warning::new("ocr_empty", "vision backend returned no text")),
        Err(error) => Err(Warning::new("vision_failed", error.to_string())),
    }
}

fn run_tesseract(image: &[u8], config: &OcrConfig) -> OcrOutcome {
    let mut child = Command::new("tesseract")
        .arg("stdin")
        .arg("stdout")
        .arg("-l")
        .arg(&config.tesseract_lang)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            Warning::new(
                "tesseract_unavailable",
                format!("could not start tesseract: {error}"),
            )
        })?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(image).map_err(|error| {
            Warning::new(
                "tesseract_input_failed",
                format!("could not send image: {error}"),
            )
        })?;
    }

    let output = child.wait_with_output().map_err(|error| {
        Warning::new(
            "tesseract_failed",
            format!("could not read OCR output: {error}"),
        )
    })?;

    if !output.status.success() {
        return Err(Warning::new(
            "tesseract_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }

    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() {
        Err(Warning::new("ocr_empty", "tesseract returned no text"))
    } else {
        Ok((text, "tesseract"))
    }
}

#[cfg(feature = "ocr-ocrs")]
fn run_ocrs(image: &[u8], config: &OcrConfig) -> OcrOutcome {
    let detection_path = config.ocrs_detection_model.as_ref().ok_or_else(|| {
        Warning::new(
            "ocrs_model_missing",
            "set AITOOLS_OCRS_DETECTION_MODEL or OcrConfig::ocrs_detection_model",
        )
    })?;
    let recognition_path = config.ocrs_recognition_model.as_ref().ok_or_else(|| {
        Warning::new(
            "ocrs_model_missing",
            "set AITOOLS_OCRS_RECOGNITION_MODEL or OcrConfig::ocrs_recognition_model",
        )
    })?;

    let detection_model = rten::Model::load_file(detection_path)
        .map_err(|error| Warning::new("ocrs_model_failed", error.to_string()))?;
    let recognition_model = rten::Model::load_file(recognition_path)
        .map_err(|error| Warning::new("ocrs_model_failed", error.to_string()))?;

    let engine = ocrs::OcrEngine::new(ocrs::OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })
    .map_err(|error| Warning::new("ocrs_engine_failed", error.to_string()))?;

    let decoded = image::load_from_memory(image)
        .map_err(|error| Warning::new("image_decode_failed", error.to_string()))?
        .into_rgb8();
    let source = ocrs::ImageSource::from_bytes(decoded.as_raw(), decoded.dimensions())
        .map_err(|error| Warning::new("ocrs_input_failed", error.to_string()))?;
    let input = engine
        .prepare_input(source)
        .map_err(|error| Warning::new("ocrs_input_failed", error.to_string()))?;
    let text = engine
        .get_text(&input)
        .map_err(|error| Warning::new("ocrs_failed", error.to_string()))?;

    if text.trim().is_empty() {
        Err(Warning::new("ocr_empty", "ocrs returned no text"))
    } else {
        Ok((text, "ocrs"))
    }
}

#[cfg(not(feature = "ocr-ocrs"))]
fn run_ocrs(_image: &[u8], _config: &OcrConfig) -> OcrOutcome {
    Err(Warning::new(
        "ocrs_not_compiled",
        "rebuild with --features ocr-ocrs to enable the pure-Rust OCR backend",
    ))
}
