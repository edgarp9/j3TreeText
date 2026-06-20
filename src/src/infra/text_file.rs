use std::borrow::Cow;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use encoding_rs::{EncoderResult, Encoding, EUC_KR, WINDOWS_1252};

use crate::domain::TextEncoding;
use crate::error::{AppError, IoUserMessage, TextEncodingUserMessage, TextFileTooLargeUserMessage};
use crate::platform::file_system;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16_BE_BOM: &[u8] = &[0xFE, 0xFF];
const TEMP_FILE_ATTEMPTS: u32 = 100;
pub(crate) const TEXT_FILE_BYTE_LIMIT: usize = 16 * 1024 * 1024;
const TEXT_FILE_BYTE_LIMIT_U64: u64 = TEXT_FILE_BYTE_LIMIT as u64;
const TEXT_FILE_MIB_LIMIT: usize = TEXT_FILE_BYTE_LIMIT / 1024 / 1024;
const LEGACY_ENCODE_BUFFER_SIZE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    pub content: String,
    pub encoding: TextEncoding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedTextExport {
    bytes: Vec<u8>,
}

impl EncodedTextExport {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub fn read_text_file(path: &Path, encoding: TextEncoding) -> Result<DecodedText, AppError> {
    let metadata = fs::metadata(path).map_err(|source| {
        AppError::io_with_user_message(
            "read text file metadata",
            IoUserMessage::ReadTextFile,
            source,
        )
    })?;
    ensure_regular_text_input_file(&metadata)?;
    ensure_text_file_byte_len_within_limit(metadata.len(), TextFileTooLargeUserMessage::Import)?;

    let file = File::open(path).map_err(|source| AppError::io("open text file", source))?;
    let mut reader = io::BufReader::new(file).take(TEXT_FILE_BYTE_LIMIT_U64.saturating_add(1));
    let mut bytes = Vec::with_capacity(metadata_len_capacity(metadata.len()));
    reader.read_to_end(&mut bytes).map_err(|source| {
        AppError::io_with_user_message("read text file", IoUserMessage::ReadTextFile, source)
    })?;
    ensure_text_file_byte_len_within_limit(
        bytes.len() as u64,
        TextFileTooLargeUserMessage::Import,
    )?;
    decode_text(&bytes, encoding)
}

pub fn write_text_file(path: &Path, encoding: TextEncoding, content: &str) -> Result<(), AppError> {
    match encoding {
        TextEncoding::Utf8 => {
            ensure_export_encoded_len_within_limit(content.len())?;
            write_text_byte_chunks_atomically(path, &[content.as_bytes()])
        }
        TextEncoding::Utf8WithBom => {
            let len = content
                .len()
                .checked_add(UTF8_BOM.len())
                .ok_or_else(|| text_file_too_large_error(TextFileTooLargeUserMessage::Export))?;
            ensure_export_encoded_len_within_limit(len)?;
            write_text_byte_chunks_atomically(path, &[UTF8_BOM, content.as_bytes()])
        }
        _ => {
            let export = encode_text_file_for_export(content, encoding)?;
            write_encoded_text_file(path, &export)
        }
    }
}

pub(crate) fn encode_text_file_for_export(
    content: &str,
    encoding: TextEncoding,
) -> Result<EncodedTextExport, AppError> {
    encode_text_for_export(content, encoding).map(|bytes| EncodedTextExport { bytes })
}

pub(crate) fn validate_text_file_export_encoding(
    content: &str,
    encoding: TextEncoding,
) -> Result<(), AppError> {
    match encoding {
        TextEncoding::AutoDetect => Err(AppError::text_encoding_with_user_message(
            "encode text file",
            TextEncodingUserMessage::Encode,
            encoding,
            "Auto Detect is only valid for import",
        )),
        TextEncoding::Utf8 => ensure_export_encoded_len_within_limit(content.len()),
        TextEncoding::Utf8WithBom => {
            let len = content
                .len()
                .checked_add(UTF8_BOM.len())
                .ok_or_else(|| text_file_too_large_error(TextFileTooLargeUserMessage::Export))?;
            ensure_export_encoded_len_within_limit(len)
        }
        TextEncoding::Utf16LeWithBom | TextEncoding::Utf16BeWithBom => {
            validate_utf16_export_len(content)
        }
        TextEncoding::KoreanEucKr => {
            validate_legacy_export_encoding(content, TextEncoding::KoreanEucKr, EUC_KR)
        }
        TextEncoding::Windows1252 => {
            validate_legacy_export_encoding(content, TextEncoding::Windows1252, WINDOWS_1252)
        }
    }
}

pub(crate) fn write_encoded_text_file(
    path: &Path,
    export: &EncodedTextExport,
) -> Result<(), AppError> {
    write_text_bytes_atomically(path, export.as_bytes())
}

pub(crate) fn write_encoded_text_export_to_file(
    file: &mut File,
    export: &EncodedTextExport,
) -> Result<(), AppError> {
    write_temp_text_file(file, &[export.as_bytes()])
}

fn metadata_len_capacity(len: u64) -> usize {
    usize::try_from(len)
        .ok()
        .map_or(TEXT_FILE_BYTE_LIMIT, |len| len.min(TEXT_FILE_BYTE_LIMIT))
}

pub(crate) fn ensure_regular_text_input_file(metadata: &fs::Metadata) -> Result<(), AppError> {
    if metadata.is_file() {
        return Ok(());
    }

    Err(AppError::user("텍스트 파일은 일반 파일이어야 합니다."))
}

fn ensure_text_file_byte_len_within_limit(
    len: u64,
    user_message: TextFileTooLargeUserMessage,
) -> Result<(), AppError> {
    if len <= TEXT_FILE_BYTE_LIMIT_U64 {
        return Ok(());
    }

    Err(text_file_too_large_error(user_message))
}

fn text_file_too_large_error(user_message: TextFileTooLargeUserMessage) -> AppError {
    AppError::text_file_too_large(user_message, TEXT_FILE_MIB_LIMIT)
}

fn write_text_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    write_text_byte_chunks_atomically(path, &[bytes])
}

fn write_text_byte_chunks_atomically(path: &Path, chunks: &[&[u8]]) -> Result<(), AppError> {
    let (temp_path, file) = create_temp_text_file(path)?;

    let mut file = match file_system::prepare_replacement_file(&temp_path, path, file) {
        Ok(file) => file,
        Err(source) => {
            let _ = fs::remove_file(&temp_path);
            return Err(AppError::io("preserve text file permissions", source));
        }
    };

    if let Err(error) = write_temp_text_file(&mut file, chunks) {
        drop(file);
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    drop(file);

    if let Err(source) = file_system::replace_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(AppError::io("replace text file", source));
    }

    Ok(())
}

fn create_temp_text_file(path: &Path) -> Result<(PathBuf, File), AppError> {
    let file_name = path.file_name().ok_or_else(|| {
        AppError::io(
            "create temporary text file",
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "text file path must include a file name",
            ),
        )
    })?;
    let unique = temp_file_unique_part();

    for attempt in 0..TEMP_FILE_ATTEMPTS {
        let mut temp_file_name = OsString::from(".");
        temp_file_name.push(file_name);
        temp_file_name.push(format!(".j3treetext-{unique}-{attempt}.partial"));
        let temp_path = path.with_file_name(temp_file_name);

        match file_system::create_replacement_file(&temp_path, path) {
            Ok(file) => return Ok((temp_path, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(AppError::io("create temporary text file", source)),
        }
    }

    Err(AppError::io(
        "create temporary text file",
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique temporary text file",
        ),
    ))
}

fn temp_file_unique_part() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{timestamp}", process::id())
}

fn write_temp_text_file(file: &mut File, chunks: &[&[u8]]) -> Result<(), AppError> {
    for chunk in chunks {
        file.write_all(chunk).map_err(|source| {
            AppError::io_with_user_message("write text file", IoUserMessage::WriteTextFile, source)
        })?;
    }
    file.flush()
        .map_err(|source| AppError::io("flush text file", source))?;
    file.sync_all()
        .map_err(|source| AppError::io("sync text file", source))?;
    Ok(())
}

pub fn decode_text(bytes: &[u8], encoding: TextEncoding) -> Result<DecodedText, AppError> {
    match encoding {
        TextEncoding::AutoDetect => decode_auto_detect(bytes),
        TextEncoding::Utf8 => decode_utf8(bytes, TextEncoding::Utf8).map(|content| DecodedText {
            content,
            encoding: TextEncoding::Utf8,
        }),
        TextEncoding::Utf8WithBom => {
            let bytes = strip_required_bom(bytes, UTF8_BOM, TextEncoding::Utf8WithBom)?;
            decode_utf8(bytes, TextEncoding::Utf8WithBom).map(|content| DecodedText {
                content,
                encoding: TextEncoding::Utf8WithBom,
            })
        }
        TextEncoding::Utf16LeWithBom => {
            let bytes = strip_required_bom(bytes, UTF16_LE_BOM, TextEncoding::Utf16LeWithBom)?;
            decode_utf16(bytes, TextEncoding::Utf16LeWithBom, Endianness::Little).map(|content| {
                DecodedText {
                    content,
                    encoding: TextEncoding::Utf16LeWithBom,
                }
            })
        }
        TextEncoding::Utf16BeWithBom => {
            let bytes = strip_required_bom(bytes, UTF16_BE_BOM, TextEncoding::Utf16BeWithBom)?;
            decode_utf16(bytes, TextEncoding::Utf16BeWithBom, Endianness::Big).map(|content| {
                DecodedText {
                    content,
                    encoding: TextEncoding::Utf16BeWithBom,
                }
            })
        }
        TextEncoding::KoreanEucKr => {
            decode_legacy(bytes, TextEncoding::KoreanEucKr, EUC_KR).map(|content| DecodedText {
                content,
                encoding: TextEncoding::KoreanEucKr,
            })
        }
        TextEncoding::Windows1252 => decode_legacy(bytes, TextEncoding::Windows1252, WINDOWS_1252)
            .map(|content| DecodedText {
                content,
                encoding: TextEncoding::Windows1252,
            }),
    }
}

pub fn encode_text(content: &str, encoding: TextEncoding) -> Result<Vec<u8>, AppError> {
    match encoding {
        TextEncoding::AutoDetect => Err(AppError::text_encoding_with_user_message(
            "encode text file",
            TextEncodingUserMessage::Encode,
            encoding,
            "Auto Detect is only valid for import",
        )),
        TextEncoding::Utf8 => Ok(content.as_bytes().to_vec()),
        TextEncoding::Utf8WithBom => {
            let mut bytes = Vec::with_capacity(UTF8_BOM.len() + content.len());
            bytes.extend_from_slice(UTF8_BOM);
            bytes.extend_from_slice(content.as_bytes());
            Ok(bytes)
        }
        TextEncoding::Utf16LeWithBom => Ok(encode_utf16(
            content,
            TextEncoding::Utf16LeWithBom,
            Endianness::Little,
        )),
        TextEncoding::Utf16BeWithBom => Ok(encode_utf16(
            content,
            TextEncoding::Utf16BeWithBom,
            Endianness::Big,
        )),
        TextEncoding::KoreanEucKr => encode_legacy(content, TextEncoding::KoreanEucKr, EUC_KR),
        TextEncoding::Windows1252 => {
            encode_legacy(content, TextEncoding::Windows1252, WINDOWS_1252)
        }
    }
}

fn encode_text_for_export(content: &str, encoding: TextEncoding) -> Result<Vec<u8>, AppError> {
    match encoding {
        TextEncoding::AutoDetect => Err(AppError::text_encoding_with_user_message(
            "encode text file",
            TextEncodingUserMessage::Encode,
            encoding,
            "Auto Detect is only valid for import",
        )),
        TextEncoding::Utf8 => {
            ensure_export_encoded_len_within_limit(content.len())?;
            Ok(content.as_bytes().to_vec())
        }
        TextEncoding::Utf8WithBom => {
            let len = content
                .len()
                .checked_add(UTF8_BOM.len())
                .ok_or_else(|| text_file_too_large_error(TextFileTooLargeUserMessage::Export))?;
            ensure_export_encoded_len_within_limit(len)?;

            let mut bytes = Vec::with_capacity(len);
            bytes.extend_from_slice(UTF8_BOM);
            bytes.extend_from_slice(content.as_bytes());
            Ok(bytes)
        }
        TextEncoding::Utf16LeWithBom => {
            encode_utf16_for_export(content, TextEncoding::Utf16LeWithBom, Endianness::Little)
        }
        TextEncoding::Utf16BeWithBom => {
            encode_utf16_for_export(content, TextEncoding::Utf16BeWithBom, Endianness::Big)
        }
        TextEncoding::KoreanEucKr => {
            encode_legacy_for_export(content, TextEncoding::KoreanEucKr, EUC_KR)
        }
        TextEncoding::Windows1252 => {
            encode_legacy_for_export(content, TextEncoding::Windows1252, WINDOWS_1252)
        }
    }
}

fn ensure_export_encoded_len_within_limit(len: usize) -> Result<(), AppError> {
    if len <= TEXT_FILE_BYTE_LIMIT {
        return Ok(());
    }

    Err(text_file_too_large_error(
        TextFileTooLargeUserMessage::Export,
    ))
}

fn decode_auto_detect(bytes: &[u8]) -> Result<DecodedText, AppError> {
    if bytes.starts_with(UTF8_BOM) {
        return decode_text(bytes, TextEncoding::Utf8WithBom);
    }
    if bytes.starts_with(UTF16_LE_BOM) {
        return decode_text(bytes, TextEncoding::Utf16LeWithBom);
    }
    if bytes.starts_with(UTF16_BE_BOM) {
        return decode_text(bytes, TextEncoding::Utf16BeWithBom);
    }

    if let Ok(content) = std::str::from_utf8(bytes) {
        return Ok(DecodedText {
            content: content.to_owned(),
            encoding: TextEncoding::Utf8,
        });
    }

    // encoding_rs follows the WHATWG EUC-KR decoder, which includes common Windows-949 extensions.
    if let Some(content) = decode_legacy_without_replacement(bytes, EUC_KR) {
        return Ok(DecodedText {
            content: content.into_owned(),
            encoding: TextEncoding::KoreanEucKr,
        });
    }

    decode_legacy(bytes, TextEncoding::Windows1252, WINDOWS_1252).map(|content| DecodedText {
        content,
        encoding: TextEncoding::Windows1252,
    })
}

fn strip_required_bom<'a>(
    bytes: &'a [u8],
    bom: &[u8],
    encoding: TextEncoding,
) -> Result<&'a [u8], AppError> {
    bytes.strip_prefix(bom).ok_or_else(|| {
        AppError::text_encoding(
            "decode text file",
            encoding,
            format!("missing {} byte order mark", encoding.display_name()),
        )
    })
}

fn decode_utf8(bytes: &[u8], encoding: TextEncoding) -> Result<String, AppError> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|source| AppError::text_encoding("decode text file", encoding, source.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Endianness {
    Little,
    Big,
}

fn decode_utf16(
    bytes: &[u8],
    encoding: TextEncoding,
    endianness: Endianness,
) -> Result<String, AppError> {
    let mut chunks = bytes.chunks_exact(2);
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in &mut chunks {
        let unit = match endianness {
            Endianness::Little => u16::from_le_bytes([chunk[0], chunk[1]]),
            Endianness::Big => u16::from_be_bytes([chunk[0], chunk[1]]),
        };
        units.push(unit);
    }

    if !chunks.remainder().is_empty() {
        return Err(AppError::text_encoding(
            "decode text file",
            encoding,
            "UTF-16 content has an odd byte length after the BOM",
        ));
    }

    String::from_utf16(&units)
        .map_err(|source| AppError::text_encoding("decode text file", encoding, source.to_string()))
}

fn encode_utf16(content: &str, encoding: TextEncoding, endianness: Endianness) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2 + content.len() * 2);
    match endianness {
        Endianness::Little => bytes.extend_from_slice(UTF16_LE_BOM),
        Endianness::Big => bytes.extend_from_slice(UTF16_BE_BOM),
    }

    for unit in content.encode_utf16() {
        let encoded = match endianness {
            Endianness::Little => unit.to_le_bytes(),
            Endianness::Big => unit.to_be_bytes(),
        };
        bytes.extend_from_slice(&encoded);
    }

    debug_assert!(
        encoding == TextEncoding::Utf16LeWithBom || encoding == TextEncoding::Utf16BeWithBom
    );
    bytes
}

fn encode_utf16_for_export(
    content: &str,
    encoding: TextEncoding,
    endianness: Endianness,
) -> Result<Vec<u8>, AppError> {
    let capacity = 2usize
        .saturating_add(content.len().saturating_mul(2))
        .min(TEXT_FILE_BYTE_LIMIT);
    let mut bytes = Vec::with_capacity(capacity);
    match endianness {
        Endianness::Little => bytes.extend_from_slice(UTF16_LE_BOM),
        Endianness::Big => bytes.extend_from_slice(UTF16_BE_BOM),
    }

    for unit in content.encode_utf16() {
        if bytes.len().saturating_add(2) > TEXT_FILE_BYTE_LIMIT {
            return Err(text_file_too_large_error(
                TextFileTooLargeUserMessage::Export,
            ));
        }

        let encoded = match endianness {
            Endianness::Little => unit.to_le_bytes(),
            Endianness::Big => unit.to_be_bytes(),
        };
        bytes.extend_from_slice(&encoded);
    }

    debug_assert!(
        encoding == TextEncoding::Utf16LeWithBom || encoding == TextEncoding::Utf16BeWithBom
    );
    Ok(bytes)
}

fn validate_utf16_export_len(content: &str) -> Result<(), AppError> {
    let mut len = 2usize;
    for _ in content.encode_utf16() {
        len = len
            .checked_add(2)
            .ok_or_else(|| text_file_too_large_error(TextFileTooLargeUserMessage::Export))?;
        if len > TEXT_FILE_BYTE_LIMIT {
            return Err(text_file_too_large_error(
                TextFileTooLargeUserMessage::Export,
            ));
        }
    }
    Ok(())
}

fn decode_legacy(
    bytes: &[u8],
    encoding: TextEncoding,
    decoder: &'static Encoding,
) -> Result<String, AppError> {
    let decoded = decode_legacy_without_replacement(bytes, decoder).ok_or_else(|| {
        AppError::text_encoding(
            "decode text file",
            encoding,
            "legacy decoder reported malformed byte sequences",
        )
    })?;

    Ok(decoded.into_owned())
}

fn decode_legacy_without_replacement<'a>(
    bytes: &'a [u8],
    decoder: &'static Encoding,
) -> Option<Cow<'a, str>> {
    decoder.decode_without_bom_handling_and_without_replacement(bytes)
}

fn encode_legacy(
    content: &str,
    encoding: TextEncoding,
    encoder: &'static Encoding,
) -> Result<Vec<u8>, AppError> {
    let (encoded, _, had_errors) = encoder.encode(content);
    if had_errors {
        return Err(legacy_encode_error(encoding));
    }

    Ok(encoded.into_owned())
}

fn encode_legacy_for_export(
    content: &str,
    encoding: TextEncoding,
    encoder: &'static Encoding,
) -> Result<Vec<u8>, AppError> {
    let mut encoder = encoder.new_encoder();
    let mut remaining = content;
    let mut bytes = Vec::with_capacity(content.len().min(TEXT_FILE_BYTE_LIMIT));
    let mut output_over_limit = false;
    let mut buffer = [0u8; LEGACY_ENCODE_BUFFER_SIZE];

    loop {
        let (result, read, written) =
            encoder.encode_from_utf8_without_replacement(remaining, &mut buffer, true);

        if !output_over_limit {
            match bytes.len().checked_add(written) {
                Some(len) if len <= TEXT_FILE_BYTE_LIMIT => {
                    bytes.extend_from_slice(&buffer[..written]);
                }
                _ => {
                    let available = TEXT_FILE_BYTE_LIMIT.saturating_sub(bytes.len());
                    bytes.extend_from_slice(&buffer[..available]);
                    output_over_limit = true;
                }
            }
        }

        remaining = &remaining[read..];

        match result {
            EncoderResult::InputEmpty => {
                return if output_over_limit {
                    Err(text_file_too_large_error(
                        TextFileTooLargeUserMessage::Export,
                    ))
                } else {
                    Ok(bytes)
                };
            }
            EncoderResult::OutputFull => {}
            EncoderResult::Unmappable(_) => return Err(legacy_encode_error(encoding)),
        }
    }
}

fn validate_legacy_export_encoding(
    content: &str,
    encoding: TextEncoding,
    encoder: &'static Encoding,
) -> Result<(), AppError> {
    let mut encoder = encoder.new_encoder();
    let mut remaining = content;
    let mut encoded_len = 0usize;
    let mut output_over_limit = false;
    let mut buffer = [0u8; LEGACY_ENCODE_BUFFER_SIZE];

    loop {
        let (result, read, written) =
            encoder.encode_from_utf8_without_replacement(remaining, &mut buffer, true);

        if !output_over_limit {
            match encoded_len.checked_add(written) {
                Some(len) if len <= TEXT_FILE_BYTE_LIMIT => encoded_len = len,
                _ => output_over_limit = true,
            }
        }

        remaining = &remaining[read..];

        match result {
            EncoderResult::InputEmpty => {
                return if output_over_limit {
                    Err(text_file_too_large_error(
                        TextFileTooLargeUserMessage::Export,
                    ))
                } else {
                    Ok(())
                };
            }
            EncoderResult::OutputFull => {}
            EncoderResult::Unmappable(_) => return Err(legacy_encode_error(encoding)),
        }
    }
}

fn legacy_encode_error(encoding: TextEncoding) -> AppError {
    AppError::text_encoding_with_user_message(
        "encode text file",
        TextEncodingUserMessage::Encode,
        encoding,
        "text contains characters that cannot be represented in the selected encoding",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_without_bom_round_trips() -> Result<(), AppError> {
        let content = "Hello\nUTF-8 한글";

        let bytes = encode_text(content, TextEncoding::Utf8)?;
        let decoded = decode_text(&bytes, TextEncoding::Utf8)?;

        assert_eq!(bytes, content.as_bytes());
        assert_eq!(decoded.content, content);
        assert_eq!(decoded.encoding, TextEncoding::Utf8);
        Ok(())
    }

    #[test]
    fn utf8_with_bom_round_trips() -> Result<(), AppError> {
        let content = "Hello with BOM";

        let bytes = encode_text(content, TextEncoding::Utf8WithBom)?;
        let decoded = decode_text(&bytes, TextEncoding::Utf8WithBom)?;

        assert!(bytes.starts_with(UTF8_BOM));
        assert_eq!(decoded.content, content);
        assert_eq!(decoded.encoding, TextEncoding::Utf8WithBom);
        Ok(())
    }

    #[test]
    fn utf16_le_and_be_with_bom_decode() -> Result<(), AppError> {
        let content = "A한";

        let le = encode_text(content, TextEncoding::Utf16LeWithBom)?;
        let be = encode_text(content, TextEncoding::Utf16BeWithBom)?;

        assert_eq!(
            decode_text(&le, TextEncoding::Utf16LeWithBom)?.content,
            content
        );
        assert_eq!(
            decode_text(&be, TextEncoding::Utf16BeWithBom)?.content,
            content
        );
        Ok(())
    }

    #[test]
    fn utf16_export_includes_required_bom() -> Result<(), AppError> {
        let content = "BOM";

        let le = encode_text(content, TextEncoding::Utf16LeWithBom)?;
        let be = encode_text(content, TextEncoding::Utf16BeWithBom)?;

        assert!(le.starts_with(UTF16_LE_BOM));
        assert!(be.starts_with(UTF16_BE_BOM));
        Ok(())
    }

    #[test]
    fn auto_detect_falls_back_after_invalid_utf8() -> Result<(), AppError> {
        let korean = encode_text("한글", TextEncoding::KoreanEucKr)?;

        let decoded = decode_text(&korean, TextEncoding::AutoDetect)?;

        assert_eq!(decoded.content, "한글");
        assert_eq!(decoded.encoding, TextEncoding::KoreanEucKr);
        Ok(())
    }

    #[test]
    fn auto_detect_uses_windows_1252_after_invalid_euc_kr() -> Result<(), AppError> {
        let decoded = decode_text(&[0xff], TextEncoding::AutoDetect)?;

        assert_eq!(decoded.content, "\u{00ff}");
        assert_eq!(decoded.encoding, TextEncoding::Windows1252);
        Ok(())
    }

    #[test]
    fn windows_1252_round_trips_representable_text() -> Result<(), AppError> {
        let content = "Cafe \u{00e9}";

        let bytes = encode_text(content, TextEncoding::Windows1252)?;
        let decoded = decode_text(&bytes, TextEncoding::Windows1252)?;

        assert_eq!(decoded.content, content);
        Ok(())
    }

    #[test]
    fn invalid_utf8_returns_decoding_error() {
        let error = decode_text(&[0xff], TextEncoding::Utf8);

        assert!(matches!(error, Err(AppError::TextEncoding { .. })));
    }

    #[test]
    fn unrepresentable_legacy_export_returns_encoding_error() {
        let error = encode_text("emoji \u{1f600}", TextEncoding::Windows1252);

        assert!(matches!(error, Err(AppError::TextEncoding { .. })));
    }

    #[test]
    fn read_text_file_rejects_file_over_byte_limit() -> Result<(), AppError> {
        let dir = unique_test_dir("read-over-limit");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("import.txt");

        let result = (|| -> Result<(), AppError> {
            let file =
                File::create(&path).map_err(|source| AppError::io("create test file", source))?;
            file.set_len(TEXT_FILE_BYTE_LIMIT_U64 + 1)
                .map_err(|source| AppError::io("resize test file", source))?;

            let read_result = read_text_file(&path, TextEncoding::Utf8);
            assert!(read_result.is_err());
            Ok(())
        })();
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn read_text_file_rejects_non_regular_input_path() -> Result<(), AppError> {
        let dir = unique_test_dir("read-non-regular");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;

        let result = (|| -> Result<(), AppError> {
            let error = match read_text_file(&dir, TextEncoding::Utf8) {
                Ok(_) => return Err(AppError::user("non-regular text input should be rejected")),
                Err(error) => error,
            };
            assert_eq!(
                error.user_message(),
                "텍스트 파일은 일반 파일이어야 합니다."
            );
            Ok(())
        })();
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn write_text_file_rejects_content_over_byte_limit() -> Result<(), AppError> {
        let dir = unique_test_dir("write-over-limit");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");

        let result: Result<(), AppError> = {
            let content = "a".repeat(TEXT_FILE_BYTE_LIMIT + 1);

            let write_result = write_text_file(&path, TextEncoding::Utf8, &content);
            assert!(write_result.is_err());
            assert!(!path.exists());
            Ok(())
        };
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn write_text_file_rejects_utf16_output_over_byte_limit() -> Result<(), AppError> {
        let dir = unique_test_dir("write-utf16-over-limit");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");

        let result: Result<(), AppError> = {
            let content = "a".repeat(TEXT_FILE_BYTE_LIMIT / 2 + 1);

            let write_result = write_text_file(&path, TextEncoding::Utf16LeWithBom, &content);
            assert!(write_result.is_err());
            assert!(!path.exists());
            Ok(())
        };
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn write_text_file_rejects_legacy_output_over_byte_limit() -> Result<(), AppError> {
        let dir = unique_test_dir("write-legacy-over-limit");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");

        let result: Result<(), AppError> = {
            let content = "a".repeat(TEXT_FILE_BYTE_LIMIT + 1);

            let write_result = write_text_file(&path, TextEncoding::Windows1252, &content);
            assert!(matches!(
                write_result,
                Err(AppError::TextFileTooLarge { .. })
            ));
            assert!(!path.exists());
            Ok(())
        };
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn write_text_file_rejects_unrepresentable_legacy_content() -> Result<(), AppError> {
        let dir = unique_test_dir("write-legacy-unrepresentable");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");

        let result: Result<(), AppError> = {
            let write_result = write_text_file(&path, TextEncoding::Windows1252, "emoji \u{1f600}");
            assert!(matches!(write_result, Err(AppError::TextEncoding { .. })));
            assert!(!path.exists());
            Ok(())
        };
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn euc_kr_export_limit_uses_encoded_byte_len_not_utf8_len() -> Result<(), AppError> {
        let content = "가".repeat(TEXT_FILE_BYTE_LIMIT / 2);

        assert!(content.len() > TEXT_FILE_BYTE_LIMIT);
        assert_eq!(
            encode_text_file_for_export(&content, TextEncoding::KoreanEucKr)?
                .as_bytes()
                .len(),
            TEXT_FILE_BYTE_LIMIT
        );
        Ok(())
    }

    #[test]
    fn write_encoded_text_file_writes_prepared_export_bytes() -> Result<(), AppError> {
        let dir = unique_test_dir("write-prepared-export");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");

        let result = (|| -> Result<(), AppError> {
            let export = encode_text_file_for_export("한글", TextEncoding::KoreanEucKr)?;

            write_encoded_text_file(&path, &export)?;
            let written =
                fs::read(&path).map_err(|source| AppError::io("read test file", source))?;
            assert_eq!(written, export.as_bytes());
            Ok(())
        })();
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn write_text_file_allows_euc_kr_utf8_over_limit_when_encoded_len_fits() -> Result<(), AppError>
    {
        let dir = unique_test_dir("write-euc-kr-utf8-over-limit");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");

        let result = (|| -> Result<(), AppError> {
            let content = "가".repeat(TEXT_FILE_BYTE_LIMIT / 2);

            assert!(content.len() > TEXT_FILE_BYTE_LIMIT);
            write_text_file(&path, TextEncoding::KoreanEucKr, &content)?;
            assert_eq!(
                fs::metadata(&path)
                    .map_err(|source| AppError::io("read test file metadata", source))?
                    .len(),
                TEXT_FILE_BYTE_LIMIT_U64
            );
            Ok(())
        })();
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn write_text_file_replaces_existing_file_without_leftover_temp() -> Result<(), AppError> {
        let dir = unique_test_dir("write-replace");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");
        fs::write(&path, "old content")
            .map_err(|source| AppError::io("write test file", source))?;

        let result = (|| -> Result<(), AppError> {
            write_text_file(&path, TextEncoding::Utf8, "new content")?;

            let written = fs::read_to_string(&path)
                .map_err(|source| AppError::io("read test file", source))?;
            assert_eq!(written, "new content");
            assert_no_temp_text_files(&dir)?;
            Ok(())
        })();
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[test]
    fn write_text_file_writes_utf8_bom_before_content() -> Result<(), AppError> {
        let dir = unique_test_dir("write-utf8-bom");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");

        let result = (|| -> Result<(), AppError> {
            let content = "Hello\nUTF-8 한글";

            write_text_file(&path, TextEncoding::Utf8WithBom, content)?;
            let written =
                fs::read(&path).map_err(|source| AppError::io("read test file", source))?;
            assert!(written.starts_with(UTF8_BOM));
            assert_eq!(&written[UTF8_BOM.len()..], content.as_bytes());
            assert_no_temp_text_files(&dir)?;
            Ok(())
        })();
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    #[cfg(unix)]
    #[test]
    fn write_text_file_preserves_existing_unix_permissions() -> Result<(), AppError> {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_test_dir("write-permissions");
        fs::create_dir(&dir).map_err(|source| AppError::io("create test dir", source))?;
        let path = dir.join("export.txt");
        let expected_mode = 0o700;
        fs::write(&path, "old content")
            .map_err(|source| AppError::io("write test file", source))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(expected_mode))
            .map_err(|source| AppError::io("set test file permissions", source))?;

        let result = (|| -> Result<(), AppError> {
            write_text_file(&path, TextEncoding::Utf8, "new content")?;

            let written = fs::read_to_string(&path)
                .map_err(|source| AppError::io("read test file", source))?;
            let mode = fs::metadata(&path)
                .map_err(|source| AppError::io("read test file metadata", source))?
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(written, "new content");
            assert_eq!(mode, expected_mode);
            assert_no_temp_text_files(&dir)?;
            Ok(())
        })();
        let cleanup_result = remove_test_dir(&dir);

        result?;
        cleanup_result
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("j3treetext-{name}-{}-{timestamp}", process::id()))
    }

    fn assert_no_temp_text_files(dir: &Path) -> Result<(), AppError> {
        for entry in fs::read_dir(dir).map_err(|source| AppError::io("read test dir", source))? {
            let entry = entry.map_err(|source| AppError::io("read test dir entry", source))?;
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            assert!(
                !file_name.contains(".j3treetext-"),
                "temporary text file was not removed: {file_name}"
            );
        }
        Ok(())
    }

    fn remove_test_dir(path: &Path) -> Result<(), AppError> {
        match fs::remove_dir_all(path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(AppError::io("remove test dir", source)),
        }
    }
}
