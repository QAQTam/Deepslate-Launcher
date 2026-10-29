//! Cross-platform owner-only filesystem helpers.
//!
//! The launcher keeps private Ed25519 keys and short-lived Bridge session
//! credentials on disk. On Unix the mode bits are enough; Windows needs an
//! explicit DACL because inherited permissions can expose the files to other
//! local accounts. These helpers deliberately fail closed: if the platform
//! permissions cannot be tightened, the caller gets an error instead of a
//! readable secret.

use std::fs::{self, File};
use std::path::Path;

/// Create (or reuse) a directory readable only by the current user.
pub fn create_private_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(path).map_err(|error| error.to_string())?;
        if !path.is_dir() {
            return Err(format!("{} 不是目录", path.display()));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        fs::create_dir_all(path).map_err(|error| error.to_string())?;
        if !path.is_dir() {
            return Err(format!("{} 不是目录", path.display()));
        }
        windows_acl::set_owner_only_dacl(path)?;
        return Ok(());
    }

    #[cfg(not(any(unix, windows)))]
    {
        fs::create_dir_all(path).map_err(|error| error.to_string())?;
        if !path.is_dir() {
            return Err(format!("{} 不是目录", path.display()));
        }
        Ok(())
    }
}

/// Create a new file readable only by the current user. The path must not
/// already exist (`create_new`) so a pre-created symlink cannot be followed.
pub fn create_private_file(path: &Path) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        return fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| error.to_string());
    }

    #[cfg(windows)]
    {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        if let Err(error) = windows_acl::set_owner_only_dacl(path) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(error);
        }
        return Ok(file);
    }

    #[cfg(not(any(unix, windows)))]
    {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| error.to_string())
    }
}

#[cfg(windows)]
mod windows_acl {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HLOCAL};
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_FILE_OBJECT,
        TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, PSID,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Replace the object's DACL with a protected, single-ACE DACL that grants
    /// full access only to the current user's SID. Removing inheritance is
    /// intentional: an inherited ACE from a shared parent must not re-open the
    /// secret file.
    pub(super) fn set_owner_only_dacl(path: &Path) -> Result<(), String> {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err("无法打开当前进程令牌以设置私有 ACL".into());
        }

        let result = (|| -> Result<(), String> {
            let mut needed = 0u32;
            // The first call is expected to fail with a buffer-size error; the
            // returned length is what matters.
            unsafe {
                GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed);
            }
            if needed == 0 {
                return Err("无法获取当前用户 SID 长度".into());
            }

            // Align the allocation for TOKEN_USER, which contains pointers.
            let words = (needed as usize + size_of::<usize>() - 1) / size_of::<usize>();
            let mut buffer = vec![0usize; words];
            let mut written = needed;
            if unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    buffer.as_mut_ptr() as *mut c_void,
                    needed,
                    &mut written,
                )
            } == 0
            {
                return Err("无法获取当前用户 SID".into());
            }
            if written as usize > buffer.len() * size_of::<usize>() {
                return Err("当前用户 SID 缓冲区无效".into());
            }

            let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
            let sid: PSID = user.User.Sid;
            if sid.is_null() {
                return Err("当前用户 SID 为空".into());
            }

            let explicit = EXPLICIT_ACCESS_W {
                grfAccessPermissions: FILE_ALL_ACCESS,
                grfAccessMode: GRANT_ACCESS,
                grfInheritance: windows_sys::Win32::Security::NO_INHERITANCE,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: null_mut(),
                    MultipleTrusteeOperation: 0,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: sid as *mut u16,
                },
            };

            let mut acl: *mut ACL = null_mut();
            let acl_status =
                unsafe { SetEntriesInAclW(1, &explicit, null(), &mut acl) };
            if acl_status != 0 {
                return Err(format!("构建私有 ACL 失败 (Win32 {acl_status})"));
            }
            if acl.is_null() {
                return Err("构建私有 ACL 返回空指针".into());
            }

            let wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let status = unsafe {
                SetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    acl,
                    null(),
                )
            };
            unsafe {
                LocalFree(acl as HLOCAL);
            }
            if status != 0 {
                return Err(format!(
                    "设置 {} 的私有 ACL 失败 (Win32 {status})",
                    path.display()
                ));
            }
            Ok(())
        })();

        unsafe {
            CloseHandle(token);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use uuid::Uuid;

    fn temp_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("deepslate-secure-fs-{}", Uuid::new_v4()))
    }

    #[test]
    fn creates_owner_only_directory_and_file() {
        let root = temp_root();
        create_private_dir(&root).unwrap();
        assert!(root.is_dir());

        let path = root.join("secret.bin");
        let mut file = create_private_file(&path).unwrap();
        file.write_all(b"secret").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(fs::read(&path).unwrap(), b"secret");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        assert!(create_private_file(&path).is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
