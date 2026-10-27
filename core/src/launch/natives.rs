use super::ExtractRule;
use std::path::Path;
use zip::ZipArchive;

pub(super) fn extract_natives(
    jar_path: &Path,
    dest_dir: &Path,
    extract_rule: Option<&ExtractRule>,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(jar_path)?;
    let mut archive = ZipArchive::new(file)?;
    let mut total = 0u64;
    if archive.len() > 4096 {
        return Err("native ZIP 条目过多".into());
    }

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();

        if name.ends_with('/') {
            continue;
        }
        if let Some(rule) = extract_rule {
            if rule.exclude.iter().any(|ex| name.starts_with(ex)) {
                continue;
            }
        }
        // 默认排除 META-INF，避免把签名/清单解压到 natives 目录
        if name.starts_with("META-INF/") {
            continue;
        }

        let ext = name.rsplit('.').next().unwrap_or("");
        if ext != "dll" && ext != "so" && ext != "dylib" {
            continue;
        }

        total = total
            .checked_add(entry.size())
            .ok_or("native ZIP 大小溢出")?;
        if total > 128 * 1024 * 1024 {
            return Err("native ZIP 解压后过大".into());
        }

        let relative = entry.enclosed_name().ok_or("非法 native ZIP 路径")?;
        if relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err("非法 native ZIP 路径".into());
        }
        write_native_file(dest_dir, &relative, &mut entry)?;
    }

    Ok(())
}

#[cfg(unix)]
fn write_native_file(
    root: &Path,
    relative: &Path,
    input: &mut impl std::io::Read,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::create_dir_all(root)?;
    let mut dir = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let name = CString::new(component.as_os_str().as_bytes())?;
        if components.peek().is_some() {
            // All operations are relative to an already opened directory descriptor.
            let result = unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o755) };
            if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error.into());
                }
            }
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            dir = unsafe { std::fs::File::from_raw_fd(fd) };
        } else {
            let temp_name = CString::new(format!(".native-{}.tmp", uuid::Uuid::new_v4()))?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    temp_name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o644,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
            let result = std::io::copy(input, &mut file).and_then(|_| {
                let renamed = unsafe {
                    libc::renameat(
                        dir.as_raw_fd(),
                        temp_name.as_ptr(),
                        dir.as_raw_fd(),
                        name.as_ptr(),
                    )
                };
                if renamed < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
            if result.is_err() {
                unsafe {
                    libc::unlinkat(dir.as_raw_fd(), temp_name.as_ptr(), 0);
                }
            }
            return result.map_err(Into::into);
        }
    }
    Err("native ZIP 路径为空".into())
}

#[cfg(not(unix))]
fn write_native_file(
    root: &Path,
    relative: &Path,
    input: &mut impl std::io::Read,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(root)?;
    if std::fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err("native 目录是符号链接".into());
    }
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component.as_os_str());
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() {
                return Err("native ZIP 路径包含符号链接".into());
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    std::io::copy(input, &mut file)?;
    Ok(())
}
