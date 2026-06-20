use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use crate::error::{AppError, IoUserMessage, TextFileTooLargeUserMessage};
use crate::infra::text_file::{ensure_regular_text_input_file, TEXT_FILE_BYTE_LIMIT};

const CONTENT_BYTE_LIMIT_U64: u64 = TEXT_FILE_BYTE_LIMIT as u64;
const CONTENT_MIB_LIMIT: usize = TEXT_FILE_BYTE_LIMIT / 1024 / 1024;

pub(super) enum ContentSource {
    None,
    Literal(String),
    File(PathBuf),
    Stdin,
}

pub(super) fn read_content_source(source: ContentSource) -> Result<String, AppError> {
    match source {
        ContentSource::None => Ok(String::new()),
        ContentSource::Literal(content) => Ok(content),
        ContentSource::File(path) => {
            let metadata = fs::metadata(&path).map_err(|source| {
                AppError::io_with_user_message(
                    "read text file metadata",
                    IoUserMessage::ReadTextFile,
                    source,
                )
            })?;
            ensure_regular_text_input_file(&metadata)?;
            reject_content_byte_len_over_limit(metadata.len())?;

            let file = fs::File::open(path).map_err(|source| {
                AppError::io_with_user_message(
                    "read text file",
                    IoUserMessage::ReadTextFile,
                    source,
                )
            })?;
            read_content_from_reader(
                io::BufReader::new(file),
                "read text file",
                IoUserMessage::ReadTextFile,
            )
        }
        ContentSource::Stdin => read_content_from_reader(
            io::stdin().lock(),
            "read stdin content",
            IoUserMessage::Generic,
        ),
    }
}

fn read_content_from_reader(
    reader: impl Read,
    action: &'static str,
    user_message: IoUserMessage,
) -> Result<String, AppError> {
    let mut content = String::new();
    let mut reader = reader.take(CONTENT_BYTE_LIMIT_U64.saturating_add(1));
    reader
        .read_to_string(&mut content)
        .map_err(|source| AppError::io_with_user_message(action, user_message, source))?;
    reject_content_over_byte_limit(&content)?;
    reject_embedded_nul_content(&content)?;
    Ok(content)
}

fn reject_content_byte_len_over_limit(len: u64) -> Result<(), AppError> {
    if len > CONTENT_BYTE_LIMIT_U64 {
        return Err(content_too_large_error());
    }

    Ok(())
}

pub(super) fn reject_content_over_byte_limit(content: &str) -> Result<(), AppError> {
    reject_content_byte_len_over_limit(content.len() as u64)
}

pub(super) fn reject_combined_content_byte_len_over_limit(
    left: u64,
    right: u64,
) -> Result<(), AppError> {
    match left.checked_add(right) {
        Some(len) => reject_content_byte_len_over_limit(len),
        None => Err(content_too_large_error()),
    }
}

fn content_too_large_error() -> AppError {
    AppError::text_file_too_large(TextFileTooLargeUserMessage::Import, CONTENT_MIB_LIMIT)
}

fn reject_embedded_nul_content(content: &str) -> Result<(), AppError> {
    if content.contains('\0') {
        return Err(AppError::user(
            "본문에 NUL 문자가 포함되어 있어 저장할 수 없습니다.",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_file_source_rejects_embedded_nul() {
        let path = temp_file_path("nul-content");
        fs::write(&path, "before\0after").expect("test content file should be written");

        let result = read_content_source(ContentSource::File(path.clone()));
        let _ = fs::remove_file(&path);
        let error = result.expect_err("embedded NUL content file should be rejected");

        assert_eq!(
            error.user_message(),
            "본문에 NUL 문자가 포함되어 있어 저장할 수 없습니다."
        );
    }

    #[test]
    fn content_file_source_rejects_content_over_byte_limit() {
        let path = temp_file_path("oversized-content");
        let file = fs::File::create(&path).expect("test content file should be created");
        file.set_len(CONTENT_BYTE_LIMIT_U64 + 1)
            .expect("test content file should be sized");

        let result = read_content_source(ContentSource::File(path.clone()));
        let _ = fs::remove_file(&path);
        let error = result.expect_err("oversized content file should be rejected");

        assert_eq!(
            error.user_message(),
            format!(
                "가져올 텍스트가 너무 큽니다. {CONTENT_MIB_LIMIT}MiB 이하로 나누어 다시 시도하세요."
            )
        );
    }

    #[test]
    fn content_file_source_rejects_non_regular_input_path() -> Result<(), AppError> {
        let path = temp_file_path("non-regular-content");
        fs::create_dir(&path).map_err(|source| AppError::io("create test dir", source))?;

        let result = read_content_source(ContentSource::File(path.clone()));
        let cleanup_result =
            fs::remove_dir(&path).map_err(|source| AppError::io("remove test dir", source));
        let assert_result = match result {
            Ok(_) => Err(AppError::user(
                "non-regular content file should be rejected",
            )),
            Err(error) => {
                assert_eq!(
                    error.user_message(),
                    "텍스트 파일은 일반 파일이어야 합니다."
                );
                Ok(())
            }
        };

        assert_result?;
        cleanup_result
    }

    #[test]
    fn stdin_content_source_rejects_content_over_byte_limit() {
        let error = read_content_from_reader(
            io::repeat(b'a'),
            "read stdin content",
            IoUserMessage::Generic,
        )
        .expect_err("oversized stdin content should be rejected");

        assert_eq!(
            error.user_message(),
            format!(
                "가져올 텍스트가 너무 큽니다. {CONTENT_MIB_LIMIT}MiB 이하로 나누어 다시 시도하세요."
            )
        );
    }

    #[test]
    fn content_read_from_external_source_rejects_embedded_nul() {
        let error = reject_embedded_nul_content("before\0after")
            .expect_err("embedded NUL content should be rejected");

        assert_eq!(
            error.user_message(),
            "본문에 NUL 문자가 포함되어 있어 저장할 수 없습니다."
        );
    }

    #[test]
    fn content_read_from_external_source_accepts_text_without_nul() {
        reject_embedded_nul_content("plain text")
            .expect("content without embedded NUL should be accepted");
    }

    fn temp_file_path(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "j3treetext-cli-{label}-{}-{unique}.txt",
            std::process::id()
        ))
    }
}
