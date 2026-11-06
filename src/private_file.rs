use std::fs::File;
use std::io;
use std::path::Path;


#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(windows)]
use std::ptr::null_mut;


#[cfg(windows)]
use windows::Win32::Security::Authorization::*;
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, OwnedHandle};

#[cfg(windows)]
use windows::Win32::Foundation::*;
#[cfg(windows)]
use windows::Win32::Security::*;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::*;
#[cfg(windows)]
use windows::Win32::System::Threading::*;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;

#[cfg(unix)]
pub fn create_private_file<P: AsRef<Path>>(path: P) -> io::Result<File> {
    use std::fs::OpenOptions;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Set file mode to 0o600 (read and write for owner only)
    options.mode(0o600);
    options.open(path)
}

pub fn create_private_dir<P: AsRef<Path>>(path: P) -> io::Result<()> {
    use std::fs::DirBuilder;

    // Only unix sets the mode, so on Windows the builder is not changed.
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = DirBuilder::new();
#[cfg(unix)]
    builder.mode(0o700);
    
// #[cfg(windows)]
//     builder.set_permissions(0o700);
    match builder
        .create(path)
        {
            Ok(_) => Ok(()),
            Err(e) => {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
}

#[cfg(windows)]
pub fn create_private_file<P: AsRef<Path>>(path: P) -> io::Result<File> {
    unsafe {
        let path_ref: &Path = path.as_ref(); // convert generic P to &Path

        // Convert path to wide string
        let wide: Vec<u16> = path_ref.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
            
        // Create file first
        let handle = CreateFileW(
            windows::core::PCWSTR(wide.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_NONE,
            None,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?;

        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", std::io::Error::last_os_error())));
        }

        // SAFETY: we own the handle and no one else will close it. The file closes it when dropped, including on the
        // error returns below.
        let file = File::from_raw_handle(handle.0 as _);

        // Open process token
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        // SAFETY: we own the token handle, this closes it when dropped.
        let _token_owner = OwnedHandle::from_raw_handle(token.0 as _);

        // Get user SID
        
        let mut len = 0u32;

        // First call is expected to fail with INSUFFICIENT_BUFFER
        match GetTokenInformation(token, TokenUser, None, 0, &mut len) {
            Ok(_) =>  return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("Unexpected success from GetTokenInformation"))),
            Err(_) => {
                let err = GetLastError();
                if err != windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER {
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", err)));
                }
            },
        };
        
        
        let mut buffer = vec![0u8; len as usize];

        GetTokenInformation(token, TokenUser, Some(buffer.as_mut_ptr() as _), len, &mut len)?;
        let token_user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let user_sid = token_user.User.Sid;
        
        // Build EXPLICIT_ACCESS allowing only the current user
        let mut ea = EXPLICIT_ACCESS_W::default();
        ea.grfAccessPermissions = GENERIC_READ.0 | GENERIC_WRITE.0;
        ea.grfAccessMode = SET_ACCESS;
        ea.grfInheritance = NO_INHERITANCE;
        BuildTrusteeWithSidW(&mut ea.Trustee, user_sid);
        
        let ea_slice = [ea];
        let mut dacl: *mut ACL = null_mut();
        let rc = SetEntriesInAclW(Some(&ea_slice), None, &mut dacl);

        if rc.0 != 0 {
            return Err(io::Error::from_raw_os_error(rc.0 as i32));
        }


        // Apply DACL only (do NOT pass owner)
        let rc = SetNamedSecurityInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,   // owner stays as-is
            None,
            Some(dacl),
            None,
        );

        // The DACL is only freed once it has been applied.
        if !dacl.is_null() {
            LocalFree(HLOCAL(dacl as *mut std::ffi::c_void));
        }

        if rc.0 != 0 {
            return Err(io::Error::from_raw_os_error(rc.0 as i32));
        }

        Ok(file)
    }
}