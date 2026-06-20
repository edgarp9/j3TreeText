use std::ffi::{c_void, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListEx, BIF_RETURNONLYFSDIRS, BROWSEINFOW, GPFIDL_DEFAULT,
};

use super::i18n::ui_text;
use super::text::utf8_to_wide_null;
use crate::domain::UiLanguage;
use crate::error::AppError;

const FILE_DIALOG_BUFFER_LEN: usize = 32_768;

pub(super) unsafe fn choose_import_text_file(
    owner: HWND,
    language: UiLanguage,
) -> Result<Option<PathBuf>, AppError> {
    choose_text_file(owner, language, FileDialogMode::Open)
}

pub(super) unsafe fn choose_export_text_file(
    owner: HWND,
    language: UiLanguage,
) -> Result<Option<PathBuf>, AppError> {
    choose_text_file(owner, language, FileDialogMode::Save)
}

pub(super) unsafe fn choose_export_text_folder(
    owner: HWND,
    language: UiLanguage,
) -> Result<Option<PathBuf>, AppError> {
    let text = ui_text(language);
    let title = utf8_to_wide_null(
        "convert export folder dialog title",
        text.file_dialog_export_folder_title(),
    )?;
    let mut display_name_buffer = vec![0u16; FILE_DIALOG_BUFFER_LEN];
    let browse_info = BROWSEINFOW {
        hwndOwner: owner,
        pszDisplayName: display_name_buffer.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS,
        ..Default::default()
    };

    let item_id_list = SHBrowseForFolderW(&browse_info);
    if item_id_list.is_null() {
        return Ok(None);
    }

    let mut folder_buffer = vec![0u16; FILE_DIALOG_BUFFER_LEN];
    let folder_buffer_len = u32::try_from(folder_buffer.len()).map_err(|_| {
        AppError::platform(
            "open export folder dialog",
            "folder path buffer is too large",
        )
    })?;
    let accepted = SHGetPathFromIDListEx(
        item_id_list,
        folder_buffer.as_mut_ptr(),
        folder_buffer_len,
        GPFIDL_DEFAULT,
    );
    CoTaskMemFree(item_id_list as *const c_void);

    if accepted == 0 {
        return Err(AppError::user(text.file_dialog_error(0)));
    }

    Ok(Some(path_from_wide_buffer(&folder_buffer)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileDialogMode {
    Open,
    Save,
}

unsafe fn choose_text_file(
    owner: HWND,
    language: UiLanguage,
    mode: FileDialogMode,
) -> Result<Option<PathBuf>, AppError> {
    let text = ui_text(language);
    let mut file_buffer = vec![0u16; FILE_DIALOG_BUFFER_LEN];
    let filter = wide_filter(&[
        (text.file_filter_text(), "*.txt"),
        (text.file_filter_all(), "*.*"),
    ])?;
    let title = match mode {
        FileDialogMode::Open => text.file_dialog_import_title(),
        FileDialogMode::Save => text.file_dialog_export_title(),
    };
    let title = utf8_to_wide_null("convert file dialog title", title)?;
    let default_extension = utf8_to_wide_null("convert default extension", "txt")?;
    let n_max_file = u32::try_from(file_buffer.len())
        .map_err(|_| AppError::platform("open text file dialog", "file buffer is too large"))?;
    let flags = match mode {
        FileDialogMode::Open => {
            OFN_EXPLORER
                | OFN_FILEMUSTEXIST
                | OFN_PATHMUSTEXIST
                | OFN_HIDEREADONLY
                | OFN_NOCHANGEDIR
        }
        FileDialogMode::Save => {
            OFN_EXPLORER
                | OFN_PATHMUSTEXIST
                | OFN_OVERWRITEPROMPT
                | OFN_HIDEREADONLY
                | OFN_NOCHANGEDIR
        }
    };
    let mut open_file_name = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: file_buffer.as_mut_ptr(),
        nMaxFile: n_max_file,
        lpstrTitle: title.as_ptr(),
        Flags: flags,
        lpstrDefExt: default_extension.as_ptr(),
        ..Default::default()
    };

    let accepted = match mode {
        FileDialogMode::Open => GetOpenFileNameW(&mut open_file_name),
        FileDialogMode::Save => GetSaveFileNameW(&mut open_file_name),
    };
    if accepted == 0 {
        let code = CommDlgExtendedError();
        if code == 0 {
            return Ok(None);
        }

        return Err(AppError::user(text.file_dialog_error(code)));
    }

    Ok(Some(path_from_wide_buffer(&file_buffer)))
}

fn wide_filter(pairs: &[(&str, &str)]) -> Result<Vec<u16>, AppError> {
    let mut filter = Vec::new();
    for (label, pattern) in pairs {
        push_wide_filter_part(&mut filter, label)?;
        push_wide_filter_part(&mut filter, pattern)?;
    }
    filter.push(0);
    Ok(filter)
}

fn push_wide_filter_part(filter: &mut Vec<u16>, part: &str) -> Result<(), AppError> {
    if part.contains('\0') {
        return Err(AppError::platform(
            "build file dialog filter",
            "filter text contains an embedded NUL character",
        ));
    }

    filter.extend(part.encode_utf16());
    filter.push(0);
    Ok(())
}

fn path_from_wide_buffer(buffer: &[u16]) -> PathBuf {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..end]).into()
}
