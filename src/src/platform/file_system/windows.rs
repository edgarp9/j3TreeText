use std::ffi::c_void;
use std::fs::{self, File};
use std::io;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::Path;
use std::ptr;

const CREATE_NEW: u32 = 1;
const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
const FILE_SHARE_DELETE: u32 = 0x0000_0004;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const GENERIC_WRITE: u32 = 0x4000_0000;
const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: i32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut SecurityAttributes,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: *mut c_void,
    ) -> *mut c_void;
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetFileSecurityW(
        file_name: *const u16,
        requested_information: u32,
        security_descriptor: *mut c_void,
        length: u32,
        length_needed: *mut u32,
    ) -> i32;
}

pub(super) fn prepare_replacement_file(
    _temp_path: &Path,
    _path: &Path,
    file: File,
) -> io::Result<File> {
    Ok(file)
}

pub(super) fn create_replacement_file(temp_path: &Path, path: &Path) -> io::Result<File> {
    let temp_path_wide = path_to_wide_null(temp_path)?;
    let path_wide = path_to_wide_null(path)?;
    let mut descriptor = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Some(read_dacl_security_descriptor(&path_wide)?),
        Ok(_) => None,
        Err(source) if source.kind() == io::ErrorKind::NotFound => None,
        Err(source) => return Err(source),
    };
    let mut security_attributes = descriptor.as_mut().map(|descriptor| SecurityAttributes {
        length: mem::size_of::<SecurityAttributes>() as u32,
        security_descriptor: descriptor.as_mut_ptr().cast(),
        inherit_handle: 0,
    });
    let security_attributes = security_attributes
        .as_mut()
        .map_or(ptr::null_mut(), |attributes| {
            attributes as *mut SecurityAttributes
        });

    // SAFETY: The path pointer is NUL-terminated. The optional security descriptor buffer remains
    // alive for the duration of the call and is only used to initialize the newly created file.
    let handle = unsafe {
        CreateFileW(
            temp_path_wide.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            security_attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };

    if handle as isize == -1 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: CreateFileW returned a valid owned file handle.
        Ok(unsafe { File::from_raw_handle(handle.cast()) })
    }
}

pub(super) fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    let temp_path = path_to_wide_null(temp_path)?;
    let path = path_to_wide_null(path)?;

    // SAFETY: Both pointers reference NUL-terminated UTF-16 buffers that remain alive for the
    // duration of the call. The API does not retain the pointers after returning.
    let replaced = unsafe {
        MoveFileExW(
            temp_path.as_ptr(),
            path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if replaced == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn read_dacl_security_descriptor(path: &[u16]) -> io::Result<Vec<u8>> {
    let mut length_needed = 0;

    // SAFETY: The path pointer is NUL-terminated. A null descriptor buffer with zero length is
    // the documented way to query the required buffer size.
    let queried = unsafe {
        GetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            0,
            &mut length_needed,
        )
    };

    if queried == 0 && length_needed == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut descriptor = vec![0; length_needed as usize];
    // SAFETY: The descriptor buffer has the size requested by the previous GetFileSecurityW call.
    let read = unsafe {
        GetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
            length_needed,
            &mut length_needed,
        )
    };

    if read == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(descriptor)
    }
}

fn path_to_wide_null(path: &Path) -> io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "file path contains an embedded NUL character",
        ));
    }

    wide.push(0);
    Ok(wide)
}
