//! MCP server exposing aitools extraction and chunking over stdio.
//!
//! Scope model:
//! * `root` is the project directory and is always accessible.
//! * `grant_base` is the widest directory a grant may ever cover (never the
//!   whole base itself).
//! * `grants` live in `<root>/.aitools/grants.json`, i.e. per project. A grant
//!   obtained in one project is invisible to servers started in other projects.
//! * The bwrap wrapper binds `grant_base` read-only, so OS-level access stays
//!   limited even if the `--root` argument is edited afterwards.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rmcp::ErrorData as McpError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use sha2::{Digest, Sha256};

use aitools::chunk::{ChunkOptions, chunk};
use aitools::extract::{ExtractOptions, Limits, OcrMode, Registry};
use aitools::model::{Source, SourceMeta};
use aitools::render::{RenderFormat, render};
use aitools::{OcrConfig, Result as AiResult};

const CACHE_LIMIT: usize = 64;
const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
struct Config {
    root: PathBuf,
    grant_base: PathBuf,
    grants_path: PathBuf,
    inline: bool,
    audit_log: Option<PathBuf>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Grant {
    /// Canonical absolute path of the explicitly allowed directory or file.
    path: String,
    #[serde(default)]
    reason: Option<String>,
    granted_at: u64,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct GrantStore {
    #[serde(default)]
    grants: Vec<Grant>,
}

#[derive(Debug, Clone)]
struct AitoolsServer {
    root: PathBuf,
    grant_base: PathBuf,
    grants_path: PathBuf,
    inline: bool,
    audit_log: Option<PathBuf>,
    cache: Arc<Mutex<HashMap<String, String>>>,
}

fn is_within(path: &Path, prefix: &Path) -> bool {
    path == prefix || path.starts_with(prefix)
}

fn canonical_dir(path: &Path, label: &str) -> AiResult<PathBuf> {
    let resolved = path.canonicalize().map_err(|error| {
        aitools::Error::Other(format!(
            "{label} {} is not accessible: {error}",
            path.display()
        ))
    })?;
    if !resolved.is_dir() {
        return Err(aitools::Error::Other(format!(
            "{label} {} is not a directory",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

impl AitoolsServer {
    fn new(config: Config) -> AiResult<Self> {
        let root = canonical_dir(&config.root, "MCP root")?;
        let grant_base = canonical_dir(&config.grant_base, "grant base")?;
        if !is_within(&root, &grant_base) {
            return Err(aitools::Error::Other(format!(
                "MCP root {} must be inside grant base {}",
                root.display(),
                grant_base.display()
            )));
        }
        let grants_path = if config.grants_path.is_absolute() {
            config.grants_path
        } else {
            root.join(config.grants_path)
        };
        Ok(Self {
            root,
            grant_base,
            grants_path,
            inline: config.inline,
            audit_log: config.audit_log,
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Read grants on every call so a newly approved grant takes effect
    /// without restarting the MCP server.
    fn grants(&self) -> Vec<Grant> {
        let Ok(text) = fs::read_to_string(&self.grants_path) else {
            return Vec::new();
        };
        let Ok(store) = serde_json::from_str::<GrantStore>(&text) else {
            return Vec::new();
        };
        store
            .grants
            .into_iter()
            .filter(|grant| {
                Path::new(&grant.path)
                    .canonicalize()
                    .map(|path| {
                        path.exists()
                            && is_within(&path, &self.grant_base)
                            && !is_within(&path, &self.root)
                            && !is_within(&self.root, &path)
                    })
                    .unwrap_or(false)
            })
            .collect()
    }

    fn allowed_prefixes(&self) -> Vec<PathBuf> {
        let mut prefixes = vec![self.root.clone()];
        prefixes.extend(
            self.grants()
                .into_iter()
                .filter_map(|grant| PathBuf::from(grant.path).canonicalize().ok()),
        );
        prefixes
    }

    /// Confirm a grant. The tool itself is put behind an `ask` permission in
    /// OpenCode, so the arguments (exact path) are shown to the user first.
    fn grant_access(&self, request: &AccessRequest) -> AiResult<String> {
        if request.path.trim().is_empty() {
            return Err(aitools::Error::Other("path is empty".to_owned()));
        }
        let candidate = PathBuf::from(&request.path);
        if !candidate.is_absolute() {
            return Err(aitools::Error::Other(
                "grant path must be absolute; relative paths are already inside root".to_owned(),
            ));
        }
        let resolved = candidate.canonicalize().map_err(|error| {
            aitools::Error::Other(format!("cannot access {}: {error}", candidate.display()))
        })?;
        if !is_within(&resolved, &self.grant_base) {
            return Err(aitools::Error::Other(format!(
                "path is outside the grantable base {}: {}",
                self.grant_base.display(),
                resolved.display()
            )));
        }
        if resolved == self.grant_base {
            return Err(aitools::Error::Other(
                "refusing to grant the whole base directory; request a specific subdirectory"
                    .to_owned(),
            ));
        }
        if is_within(&resolved, &self.root) {
            return Ok(format!(
                "already allowed by the server root: {}",
                resolved.display()
            ));
        }
        if is_within(&self.root, &resolved) {
            return Err(aitools::Error::Other(
                "grant cannot be an ancestor of the server root".to_owned(),
            ));
        }

        let mut grants = self.grants();
        if grants
            .iter()
            .any(|grant| Path::new(&grant.path) == resolved)
        {
            return Ok(format!(
                "already granted for this project: {}",
                resolved.display()
            ));
        }
        grants.push(Grant {
            path: resolved.display().to_string(),
            reason: request.reason.clone(),
            granted_at: now_secs(),
        });
        let text = serde_json::to_string_pretty(&GrantStore { grants })?;
        if let Some(parent) = self.grants_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                aitools::Error::Other(format!("cannot create grant directory: {error}"))
            })?;
        }
        let temporary = self.grants_path.with_extension("json.tmp");
        fs::write(&temporary, text.as_bytes())
            .map_err(|error| aitools::Error::Other(format!("cannot write grants: {error}")))?;
        fs::rename(&temporary, &self.grants_path)
            .map_err(|error| aitools::Error::Other(format!("cannot commit grants: {error}")))?;

        Ok(format!(
            "Granted for this project only: {}\nProject scope: {}\nGrant store: {}",
            resolved.display(),
            self.root.display(),
            self.grants_path.display()
        ))
    }

    fn audit(&self, tool: &str, detail: &str, ok: bool) {
        let Some(path) = &self.audit_log else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let line = format!(
            "{}\t{tool}\t{}\t{}\n",
            now_secs(),
            ok,
            detail.replace(['\t', '\n'], " ")
        );
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// Resolve `path` inside one of the allowed prefixes, following symlinks.
    fn resolve_path(&self, path: &str) -> AiResult<(PathBuf, fs::Metadata)> {
        if path.trim().is_empty() {
            return Err(aitools::Error::Other("path is empty".to_owned()));
        }
        let candidate = Path::new(path);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.root.join(candidate)
        };
        let resolved = candidate.canonicalize().map_err(|error| {
            aitools::Error::Other(format!("cannot access {}: {error}", candidate.display()))
        })?;
        if !self
            .allowed_prefixes()
            .iter()
            .any(|prefix| is_within(&resolved, prefix))
        {
            return Err(aitools::Error::Other(format!(
                "path is outside the allowed scope: {}; ask for access with request_access",
                resolved.display()
            )));
        }
        let metadata = fs::metadata(&resolved)
            .map_err(|error| aitools::Error::Other(format!("cannot stat file: {error}")))?;
        if !metadata.is_file() {
            return Err(aitools::Error::Other(format!(
                "{} is not a regular file",
                resolved.display()
            )));
        }
        if metadata.len() > MAX_INPUT_BYTES as u64 {
            return Err(aitools::Error::LimitExceeded(format!(
                "file is {} bytes, limit is {MAX_INPUT_BYTES}",
                metadata.len()
            )));
        }
        Ok((resolved, metadata))
    }

    /// Load a file together with a version tag so edits invalidate the cache.
    fn load_file(&self, path: &str) -> AiResult<(Source, String)> {
        let (resolved, metadata) = self.resolve_path(path)?;
        let name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unnamed")
            .to_owned();
        let version = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        let bytes = fs::read(&resolved)
            .map_err(|error| aitools::Error::Other(format!("cannot read file: {error}")))?;
        Ok((
            source_from_bytes(bytes, &name),
            format!("{}:{version}:{}", resolved.display(), metadata.len()),
        ))
    }

    fn cached(&self, key: &str, produce: impl FnOnce() -> AiResult<String>) -> AiResult<String> {
        if let Some(value) = self.cache.lock().unwrap().get(key) {
            return Ok(value.clone());
        }
        let value = produce()?;
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= CACHE_LIMIT {
            if let Some(key) = cache.keys().next().cloned() {
                cache.remove(&key);
            }
        }
        cache.insert(key.to_owned(), value.clone());
        Ok(value)
    }
}

fn default_md() -> String {
    "md".to_owned()
}
fn default_ocr() -> String {
    "off".to_owned()
}
fn default_max_tokens() -> usize {
    800
}
fn default_overlap() -> usize {
    80
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ExtractRequest {
    /// Base64-encoded file bytes.
    content_b64: String,
    /// Original file name (with extension) used for format detection.
    name: String,
    /// Output format: `md` or `json`.
    #[serde(default = "default_md")]
    format: String,
    /// OCR backend: `off`, `auto`, `ocrs`, `tesseract`, or `vision`.
    #[serde(default = "default_ocr")]
    ocr: String,
    /// Include raw image bytes in JSON output.
    #[serde(default)]
    include_images: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ChunkRequest {
    /// Base64-encoded file bytes.
    content_b64: String,
    /// Original file name (with extension) used for format detection.
    name: String,
    /// Maximum tokens per chunk.
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
    /// Overlap between chunks in tokens.
    #[serde(default = "default_overlap")]
    overlap_tokens: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ExtractFileRequest {
    /// Path to the file, relative to the server root. Absolute paths are
    /// accepted when they resolve inside the root or an approved grant.
    path: String,
    /// Output format: `md` or `json`.
    #[serde(default = "default_md")]
    format: String,
    /// OCR backend: `off`, `auto`, `ocrs`, `tesseract`, or `vision`.
    #[serde(default = "default_ocr")]
    ocr: String,
    /// Include raw image bytes in JSON output.
    #[serde(default)]
    include_images: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ChunkFileRequest {
    /// Path to the file, relative to the server root. Absolute paths are
    /// accepted when they resolve inside the root or an approved grant.
    path: String,
    /// Maximum tokens per chunk.
    #[serde(default = "default_max_tokens")]
    max_tokens: usize,
    /// Overlap between chunks in tokens.
    #[serde(default = "default_overlap")]
    overlap_tokens: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AccessRequest {
    /// Absolute path to one directory (or file) to add to this project's scope.
    path: String,
    /// Why access is needed; shown in the user approval prompt.
    #[serde(default)]
    reason: Option<String>,
}

fn parse_ocr(mode: &str) -> AiResult<OcrMode> {
    match mode {
        "off" => Ok(OcrMode::Off),
        "auto" => Ok(OcrMode::Auto),
        "ocrs" => Ok(OcrMode::Ocrs),
        "tesseract" => Ok(OcrMode::Tesseract),
        "vision" => Ok(OcrMode::Vision),
        _ => Err(aitools::Error::Other(format!("unknown OCR mode: {mode}"))),
    }
}

fn source_from_bytes(bytes: Vec<u8>, name: &str) -> Source {
    let mime = infer::get(&bytes)
        .map(|kind| kind.mime_type().to_owned())
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    Source {
        meta: SourceMeta {
            name: name.to_owned(),
            mime,
            size_bytes: bytes.len() as u64,
        },
        bytes,
    }
}

fn decode(content_b64: &str, name: &str) -> AiResult<Source> {
    let bytes = BASE64
        .decode(content_b64)
        .map_err(|error| aitools::Error::Other(format!("invalid base64: {error}")))?;
    Ok(source_from_bytes(bytes, name))
}

fn cache_key(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn render_source(
    source: Source,
    format: RenderFormat,
    ocr: OcrMode,
    include_images: bool,
) -> AiResult<String> {
    let options = ExtractOptions {
        limits: Limits::default(),
        ocr,
        include_images,
        ocr_config: OcrConfig::default(),
        ..ExtractOptions::default()
    };
    let document = Registry::default().extract(source, &options)?;
    render(&document, format)
}

fn chunk_source(source: Source, max_tokens: usize, overlap_tokens: usize) -> AiResult<String> {
    let document = Registry::default().extract(source, &ExtractOptions::default())?;
    let chunks = chunk(
        &document,
        &ChunkOptions {
            max_tokens,
            overlap_tokens,
        },
    )?;
    Ok(serde_json::to_string_pretty(&chunks)?)
}

fn to_text(output: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(output)]))
}

#[tool_router]
impl AitoolsServer {
    /// Render a file already present on disk inside the configured scope.
    #[tool(
        description = "Extract a file from disk and render it as Markdown or JSON. The path must be inside the project root or a user-approved grant. Preferred over extract_document when you have a file path."
    )]
    fn extract_file(
        &self,
        Parameters(request): Parameters<ExtractFileRequest>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = (|| -> AiResult<String> {
            let format = match request.format.as_str() {
                "json" => RenderFormat::Json,
                _ => RenderFormat::Markdown,
            };
            let ocr = parse_ocr(&request.ocr)?;
            let (source, version) = self.load_file(&request.path)?;
            let key = cache_key(&[
                b"extract_file",
                version.as_bytes(),
                request.format.as_bytes(),
                request.ocr.as_bytes(),
                &[request.include_images as u8],
            ]);
            self.cached(&key, || {
                render_source(source, format, ocr, request.include_images)
            })
        })();
        let ok = outcome.is_ok();
        self.audit("extract_file", &request.path, ok);
        to_text(outcome.map_err(mcp_error)?)
    }

    /// Chunk a file already present on disk inside the configured scope.
    #[tool(
        description = "Split a file from disk into token-aware chunks. The path must be inside the project root or a user-approved grant. Returns JSON."
    )]
    fn chunk_file(
        &self,
        Parameters(request): Parameters<ChunkFileRequest>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = (|| -> AiResult<String> {
            let (source, version) = self.load_file(&request.path)?;
            let key = cache_key(&[
                b"chunk_file",
                version.as_bytes(),
                &request.max_tokens.to_be_bytes(),
                &request.overlap_tokens.to_be_bytes(),
            ]);
            self.cached(&key, || {
                chunk_source(source, request.max_tokens, request.overlap_tokens)
            })
        })();
        let ok = outcome.is_ok();
        self.audit("chunk_file", &request.path, ok);
        to_text(outcome.map_err(mcp_error)?)
    }

    /// Ask for explicit access to a path outside the current project scope.
    #[tool(
        description = "Request user approval to add ONE directory or file outside the project root to this project's scope. The OpenCode permission `aitools_request_access` must be set to `ask` so the exact path is shown to the user before this runs. Grants are stored per project in .aitools/grants.json, apply immediately, and are invisible to other projects."
    )]
    fn request_access(
        &self,
        Parameters(request): Parameters<AccessRequest>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = self.grant_access(&request);
        let ok = outcome.is_ok();
        self.audit("request_access", &request.path, ok);
        to_text(outcome.map_err(mcp_error)?)
    }

    /// Render inline bytes (useful when the client cannot share a file path).
    #[tool(
        description = "Extract base64-encoded file bytes and render as Markdown or JSON. Disabled by the sandbox wrapper; use extract_file instead when the file is available on disk."
    )]
    fn extract_document(
        &self,
        Parameters(request): Parameters<ExtractRequest>,
    ) -> Result<CallToolResult, McpError> {
        if !self.inline {
            return Err(mcp_error(aitools::Error::Other(
                "inline base64 tools are disabled; use extract_file".to_owned(),
            )));
        }
        let format = match request.format.as_str() {
            "json" => RenderFormat::Json,
            _ => RenderFormat::Markdown,
        };
        let ocr = parse_ocr(&request.ocr).map_err(mcp_error)?;
        let key = cache_key(&[
            b"extract_document",
            request.content_b64.as_bytes(),
            request.name.as_bytes(),
            request.format.as_bytes(),
            request.ocr.as_bytes(),
            &[request.include_images as u8],
        ]);

        let output = self
            .cached(&key, || {
                let source = decode(&request.content_b64, &request.name)?;
                render_source(source, format, ocr, request.include_images)
            })
            .map_err(mcp_error)?;
        to_text(output)
    }

    /// Chunk inline bytes (useful when the client cannot share a file path).
    #[tool(
        description = "Split base64-encoded file bytes into token-aware chunks. Disabled by the sandbox wrapper; use chunk_file instead when the file is available on disk. Returns JSON."
    )]
    fn chunk_document(
        &self,
        Parameters(request): Parameters<ChunkRequest>,
    ) -> Result<CallToolResult, McpError> {
        if !self.inline {
            return Err(mcp_error(aitools::Error::Other(
                "inline base64 tools are disabled; use chunk_file".to_owned(),
            )));
        }
        let key = cache_key(&[
            b"chunk_document",
            request.content_b64.as_bytes(),
            request.name.as_bytes(),
            &request.max_tokens.to_be_bytes(),
            &request.overlap_tokens.to_be_bytes(),
        ]);

        let output = self
            .cached(&key, || {
                let source = decode(&request.content_b64, &request.name)?;
                chunk_source(source, request.max_tokens, request.overlap_tokens)
            })
            .map_err(mcp_error)?;
        to_text(output)
    }

    #[tool(
        description = "List supported formats, the project root, the grantable base and the grants approved for this project."
    )]
    fn list_formats(&self) -> Result<CallToolResult, McpError> {
        let formats = [
            ("txt", "plain text"),
            ("md", "Markdown"),
            ("pdf", "PDF (text layer, optional OCR)"),
            ("docx", "Word document"),
            ("xlsx", "Excel spreadsheet (also xls, xlsm, ods)"),
            ("png", "images (also jpg, gif, webp, tiff)"),
        ];
        let mut list: Vec<String> = formats
            .iter()
            .map(|(ext, description)| format!("{ext}: {description}"))
            .collect();
        list.push(format!("root: {}", self.root.display()));
        list.push(format!("grant base: {}", self.grant_base.display()));
        list.push(format!("grant store: {}", self.grants_path.display()));
        let grants = self.grants();
        if grants.is_empty() {
            list.push("grants: none".to_owned());
        } else {
            for grant in grants {
                list.push(format!(
                    "grant: {}{}",
                    grant.path,
                    grant
                        .reason
                        .map(|reason| format!(" ({reason})"))
                        .unwrap_or_default()
                ));
            }
        }
        list.push(format!(
            "inline base64 tools: {}",
            if self.inline { "enabled" } else { "disabled" }
        ));
        to_text(list.join("\n"))
    }
}

fn mcp_error(error: aitools::Error) -> McpError {
    McpError::invalid_request(error.to_string(), None)
}

#[tool_handler]
impl ServerHandler for AitoolsServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(format!(
                "Extract text and structure from PDF, DOCX, XLSX, images and text files. \
                 Project scope: {}. Grantable base: {}. \
                 Use extract_file and chunk_file for files on disk. \
                 If a needed path is outside the project scope, call request_access with the exact path \
                 and a reason; the user must approve it. Grants are per project: another project \
                 cannot see this project's grants. list_formats shows current scope.",
                self.root.display(),
                self.grant_base.display()
            ))
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn flag_value(args: &[String], flag: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    args.iter()
        .position(|arg| arg == flag)
        .map(|index| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value").into())
        })
        .transpose()
}

fn default_grant_base(root: &Path) -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let git = PathBuf::from(home).join("Git");
        if root.starts_with(&git) && root != git {
            return git;
        }
    }
    root.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf())
}

/// Root resolution: explicit `--root`, then `AITOOLS_ROOT`, then process cwd.
fn configured_config() -> Result<Config, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = flag_value(&args, "--root")?
        .map(PathBuf::from)
        .or_else(|| env_path("AITOOLS_ROOT"))
        .map_or_else(std::env::current_dir, Ok)?;
    let grant_base = flag_value(&args, "--grant-base")?
        .map(PathBuf::from)
        .or_else(|| env_path("AITOOLS_GRANT_BASE"))
        .unwrap_or_else(|| default_grant_base(&root));
    let grants_path = flag_value(&args, "--grants")?
        .map(PathBuf::from)
        .or_else(|| env_path("AITOOLS_GRANTS"))
        .unwrap_or_else(|| root.join(".aitools/grants.json"));
    let inline = !args.iter().any(|arg| arg == "--no-inline")
        && std::env::var("AITOOLS_NO_INLINE").ok().as_deref() != Some("1");
    let audit_log = flag_value(&args, "--audit-log")?
        .map(PathBuf::from)
        .or_else(|| env_path("AITOOLS_AUDIT_LOG"));
    Ok(Config {
        root,
        grant_base,
        grants_path,
        inline,
        audit_log,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = AitoolsServer::new(configured_config()?)?
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "aitools-mcp-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("project")).unwrap();
            fs::create_dir_all(dir.join("extra")).unwrap();
            fs::write(dir.join("project/README.md"), "# project").unwrap();
            fs::write(dir.join("extra/note.txt"), "extra").unwrap();
            Self { dir }
        }

        fn config(&self) -> Config {
            Config {
                root: self.dir.join("project"),
                grant_base: self.dir.clone(),
                grants_path: self.dir.join("grants.json"),
                inline: false,
                audit_log: None,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn path_is_confined_to_root_until_granted() {
        let fixture = Fixture::new();
        let server = AitoolsServer::new(fixture.config()).unwrap();
        let (path, _) = server.resolve_path("README.md").unwrap();
        assert_eq!(
            path,
            fixture
                .dir
                .join("project/README.md")
                .canonicalize()
                .unwrap()
        );
        assert!(server.resolve_path("../extra/note.txt").is_err());
        assert!(server.resolve_path("/etc/passwd").is_err());
        assert!(server.resolve_path("").is_err());

        server
            .grant_access(&AccessRequest {
                path: fixture.dir.join("extra").display().to_string(),
                reason: Some("test".to_owned()),
            })
            .unwrap();
        let (path, _) = server.resolve_path("../extra/note.txt").unwrap();
        assert_eq!(
            path,
            fixture.dir.join("extra/note.txt").canonicalize().unwrap()
        );
    }

    #[test]
    fn grants_cannot_widen_to_the_whole_base_or_outside_it() {
        let fixture = Fixture::new();
        let server = AitoolsServer::new(fixture.config()).unwrap();
        assert!(
            server
                .grant_access(&AccessRequest {
                    path: fixture.dir.display().to_string(),
                    reason: None,
                })
                .is_err()
        );
        assert!(
            server
                .grant_access(&AccessRequest {
                    path: "/etc".to_owned(),
                    reason: None,
                })
                .is_err()
        );
        assert!(
            server
                .grant_access(&AccessRequest {
                    path: "relative".to_owned(),
                    reason: None,
                })
                .is_err()
        );
        assert!(server.grants().is_empty());
    }

    #[test]
    fn inline_tools_can_be_disabled() {
        let fixture = Fixture::new();
        let server = AitoolsServer::new(fixture.config()).unwrap();
        let request = ExtractRequest {
            content_b64: BASE64.encode("hello"),
            name: "hello.txt".to_owned(),
            format: "md".to_owned(),
            ocr: "off".to_owned(),
            include_images: false,
        };
        assert!(server.extract_document(Parameters(request)).is_err());
    }

    #[test]
    fn file_loading_carries_a_version() {
        let fixture = Fixture::new();
        let server = AitoolsServer::new(fixture.config()).unwrap();
        let (source, version) = server.load_file("README.md").unwrap();
        assert_eq!(source.meta.name, "README.md");
        assert!(!source.bytes.is_empty());
        assert!(version.contains("README.md"));
    }
}
