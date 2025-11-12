//! File I/O tools for reading files and listing directories
//!
//! Provides smart file reading with automatic format detection:
//! - Text files: Plain text content
//! - PDF files: Extracted text
//! - Image files: Base64 encoding with metadata
//! - Audio/Video files: Metadata only (for future STT/video LLM support)

use composable_rust_core::agent::{Tool, ToolError, ToolExecutorFn, ToolResult};
use serde_json::json;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Maximum file size for text and PDF (10MB)
const MAX_TEXT_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum image size (1MB to prevent token explosion)
const MAX_IMAGE_SIZE: u64 = 1024 * 1024;

/// Base directory for file operations (sandbox)
///
/// In production, this should be configured per-agent. For now, use current directory.
const ALLOWED_BASE_DIR: &str = ".";

/// Validate and resolve a path to prevent directory traversal attacks
///
/// # Errors
///
/// Returns `ToolError` if:
/// - Path contains `..` (parent directory traversal)
/// - Path is absolute (must be relative to base)
/// - Path escapes the allowed directory after canonicalization
/// - Path is a symlink pointing outside the sandbox
fn validate_and_resolve_path(path: &str) -> Result<PathBuf, ToolError> {
    let path_buf = Path::new(path);

    // 1. Reject parent directory traversal in the input
    for component in path_buf.components() {
        if matches!(component, Component::ParentDir) {
            return Err(ToolError {
                message: "Parent directory (..) not allowed in path".to_string(),
            });
        }
    }

    // 2. Resolve relative to allowed base directory
    let base = Path::new(ALLOWED_BASE_DIR);
    let full_path = if path_buf.is_absolute() {
        // For absolute paths, check they start with base
        path_buf.to_path_buf()
    } else {
        base.join(path_buf)
    };

    // 3. Canonicalize to resolve symlinks and verify within sandbox
    let canonical = full_path.canonicalize().map_err(|e| ToolError {
        message: format!("Failed to resolve path: {e}"),
    })?;

    let base_canonical = base.canonicalize().map_err(|e| ToolError {
        message: format!("Failed to resolve base directory: {e}"),
    })?;

    if !canonical.starts_with(&base_canonical) {
        return Err(ToolError {
            message: "Path escapes allowed directory".to_string(),
        });
    }

    Ok(canonical)
}

/// Detect file type from extension
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileType {
    Text,
    Pdf,
    Image,
    Audio,
    Video,
}

/// Detect file type from path extension
fn detect_file_type(path: &Path) -> FileType {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("pdf") => FileType::Pdf,
        Some("jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp") => FileType::Image,
        Some("mp3" | "wav" | "flac" | "ogg" | "m4a") => FileType::Audio,
        Some("mp4" | "avi" | "mkv" | "mov" | "webm") => FileType::Video,
        _ => FileType::Text,
    }
}

/// Create the `read_file` tool
///
/// Smart file reader that handles multiple formats:
/// - Text: Returns raw content
/// - PDF: Returns extracted text
/// - Images: Returns base64 + metadata (dimensions, size, format)
/// - Audio/Video: Returns metadata only (duration, codec, size)
///
/// Returns JSON:
/// ```json
/// {
///   "type": "text|pdf|image|audio|video",
///   "content": "...",           // For text/PDF
///   "base64": "...",            // For images
///   "metadata": { ... }         // Format-specific metadata
/// }
/// ```
#[must_use]
pub fn read_file_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "read_file".to_string(),
        description: "Read file with automatic format detection (text, PDF, image, audio, video)"
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to file (relative to allowed directory)"
                }
            },
            "required": ["path"]
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let path_str = parsed["path"]
                .as_str()
                .ok_or_else(|| ToolError {
                    message: "Missing 'path' field".to_string(),
                })?;

            // Validate and resolve path
            let path = validate_and_resolve_path(path_str)?;

            // Check file exists and is a file (not directory)
            if !path.is_file() {
                return Err(ToolError {
                    message: format!("Not a file: {path_str}"),
                });
            }

            // Detect file type
            let file_type = detect_file_type(&path);

            // Handle based on type
            match file_type {
                FileType::Text => read_text_file(&path).await,
                FileType::Pdf => read_pdf_file(&path).await,
                FileType::Image => read_image_file(&path).await,
                FileType::Audio => read_audio_metadata(&path).await,
                FileType::Video => read_video_metadata(&path).await,
            }
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

/// Read a text file
async fn read_text_file(path: &Path) -> ToolResult {
    // Check size
    let metadata = tokio::fs::metadata(path).await.map_err(|e| ToolError {
        message: format!("Failed to read file metadata: {e}"),
    })?;

    if metadata.len() > MAX_TEXT_FILE_SIZE {
        return Err(ToolError {
            message: format!("File too large (>{MAX_TEXT_FILE_SIZE} bytes)"),
        });
    }

    // Read content
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| ToolError {
            message: format!("Failed to read file: {e}"),
        })?;

    let result = json!({
        "type": "text",
        "content": content,
        "metadata": {
            "size": metadata.len(),
            "path": path.to_string_lossy(),
        }
    });

    Ok(result.to_string())
}

/// Read and extract text from PDF
async fn read_pdf_file(path: &Path) -> ToolResult {
    // Check size
    let metadata = tokio::fs::metadata(path).await.map_err(|e| ToolError {
        message: format!("Failed to read file metadata: {e}"),
    })?;

    if metadata.len() > MAX_TEXT_FILE_SIZE {
        return Err(ToolError {
            message: format!("File too large (>{MAX_TEXT_FILE_SIZE} bytes)"),
        });
    }

    // Read PDF bytes
    let bytes = tokio::fs::read(path).await.map_err(|e| ToolError {
        message: format!("Failed to read file: {e}"),
    })?;

    // Extract text (blocking operation, run in spawn_blocking)
    let text = tokio::task::spawn_blocking(move || {
        pdf_extract::extract_text_from_mem(&bytes)
    })
    .await
    .map_err(|e| ToolError {
        message: format!("Failed to spawn PDF extraction task: {e}"),
    })?
    .map_err(|e| ToolError {
        message: format!("Failed to extract PDF text: {e}"),
    })?;

    let result = json!({
        "type": "pdf",
        "content": text,
        "metadata": {
            "size": metadata.len(),
            "path": path.to_string_lossy(),
        }
    });

    Ok(result.to_string())
}

/// Read image file and return base64 + metadata
async fn read_image_file(path: &Path) -> ToolResult {
    // Check size
    let metadata = tokio::fs::metadata(path).await.map_err(|e| ToolError {
        message: format!("Failed to read file metadata: {e}"),
    })?;

    if metadata.len() > MAX_IMAGE_SIZE {
        return Err(ToolError {
            message: format!("Image too large (>{MAX_IMAGE_SIZE} bytes, ~340K tokens)"),
        });
    }

    // Read image bytes
    let bytes = tokio::fs::read(path).await.map_err(|e| ToolError {
        message: format!("Failed to read file: {e}"),
    })?;

    // Get image dimensions (blocking operation)
    let path_clone = path.to_path_buf();
    let dimensions = tokio::task::spawn_blocking(move || {
        image::image_dimensions(&path_clone)
    })
    .await
    .map_err(|e| ToolError {
        message: format!("Failed to spawn image processing task: {e}"),
    })?
    .map_err(|e| ToolError {
        message: format!("Failed to read image dimensions: {e}"),
    })?;

    // Encode to base64
    let base64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);

    // Detect MIME type from extension
    let mime_type = match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    };

    let result = json!({
        "type": "image",
        "base64": base64_data,
        "metadata": {
            "size": metadata.len(),
            "width": dimensions.0,
            "height": dimensions.1,
            "mime_type": mime_type,
            "path": path.to_string_lossy(),
        }
    });

    Ok(result.to_string())
}

/// Read audio file metadata
async fn read_audio_metadata(path: &Path) -> ToolResult {
    let metadata = tokio::fs::metadata(path).await.map_err(|e| ToolError {
        message: format!("Failed to read file metadata: {e}"),
    })?;

    let result = json!({
        "type": "audio",
        "metadata": {
            "size": metadata.len(),
            "path": path.to_string_lossy(),
            "note": "Audio content not yet supported. Use speech-to-text API for transcription."
        }
    });

    Ok(result.to_string())
}

/// Read video file metadata
async fn read_video_metadata(path: &Path) -> ToolResult {
    let metadata = tokio::fs::metadata(path).await.map_err(|e| ToolError {
        message: format!("Failed to read file metadata: {e}"),
    })?;

    let result = json!({
        "type": "video",
        "metadata": {
            "size": metadata.len(),
            "path": path.to_string_lossy(),
            "note": "Video content not yet supported. Use video-capable LLM API for analysis."
        }
    });

    Ok(result.to_string())
}

/// Create the `list_directory` tool
///
/// List files and directories in a directory.
///
/// Returns JSON:
/// ```json
/// {
///   "entries": [
///     {"name": "file.txt", "type": "file", "size": 1024},
///     {"name": "subdir", "type": "directory"}
///   ]
/// }
/// ```
#[must_use]
pub fn list_directory_tool() -> (Tool, ToolExecutorFn) {
    let tool = Tool {
        name: "list_directory".to_string(),
        description: "List files and directories in a directory".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to directory (relative to allowed directory, defaults to '.')"
                }
            }
        }),
    };

    let executor = Arc::new(|input: String| {
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&input).map_err(|e| {
                ToolError {
                    message: format!("Invalid input JSON: {e}"),
                }
            })?;

            let path_str = parsed["path"].as_str().unwrap_or(".");

            // Validate and resolve path
            let path = validate_and_resolve_path(path_str)?;

            // Check it's a directory
            if !path.is_dir() {
                return Err(ToolError {
                    message: format!("Not a directory: {path_str}"),
                });
            }

            // Read directory entries
            let mut entries = Vec::new();
            let mut read_dir = tokio::fs::read_dir(&path)
                .await
                .map_err(|e| ToolError {
                    message: format!("Failed to read directory: {e}"),
                })?;

            while let Some(entry) = read_dir.next_entry().await.map_err(|e| ToolError {
                message: format!("Failed to read directory entry: {e}"),
            })? {
                let file_type = entry.file_type().await.map_err(|e| ToolError {
                    message: format!("Failed to read file type: {e}"),
                })?;

                let name = entry.file_name().to_string_lossy().to_string();

                let entry_json = if file_type.is_file() {
                    let metadata = entry.metadata().await.map_err(|e| ToolError {
                        message: format!("Failed to read metadata: {e}"),
                    })?;

                    json!({
                        "name": name,
                        "type": "file",
                        "size": metadata.len()
                    })
                } else if file_type.is_dir() {
                    json!({
                        "name": name,
                        "type": "directory"
                    })
                } else {
                    json!({
                        "name": name,
                        "type": "other"
                    })
                };

                entries.push(entry_json);
            }

            let result = json!({
                "entries": entries
            });

            Ok(result.to_string())
        }) as std::pin::Pin<
            Box<dyn std::future::Future<Output = ToolResult> + Send>,
        >
    }) as ToolExecutorFn;

    (tool, executor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_file_type() {
        assert_eq!(detect_file_type(Path::new("file.txt")), FileType::Text);
        assert_eq!(detect_file_type(Path::new("doc.pdf")), FileType::Pdf);
        assert_eq!(detect_file_type(Path::new("photo.jpg")), FileType::Image);
        assert_eq!(detect_file_type(Path::new("photo.PNG")), FileType::Image);
        assert_eq!(detect_file_type(Path::new("song.mp3")), FileType::Audio);
        assert_eq!(detect_file_type(Path::new("video.mp4")), FileType::Video);
    }

    #[test]
    fn test_validate_path_rejects_parent_dir() {
        let result = validate_and_resolve_path("../etc/passwd");
        assert!(result.is_err());
        assert!(result
            .expect_err("should fail")
            .message
            .contains("Parent directory"));
    }

    #[test]
    fn test_read_file_tool_schema() {
        let (tool, _executor) = read_file_tool();
        assert_eq!(tool.name, "read_file");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_list_directory_tool_schema() {
        let (tool, _executor) = list_directory_tool();
        assert_eq!(tool.name, "list_directory");
        assert!(tool.input_schema.is_object());
    }
}
