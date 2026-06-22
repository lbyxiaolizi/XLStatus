#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{bail, Context, Result};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use tokio::fs;

/// File entry information
#[allow(dead_code)]
#[derive(Debug)]
pub struct FileEntry {
    pub name: String,
    pub file_type: FileType,
    pub size: u64,
    pub mode: u32,
    pub modified_at: i64,
    pub symlink_target: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum FileType {
    File,
    Dir,
    Symlink,
}

/// List files in a directory
#[allow(dead_code)]
pub async fn list_files(path: &str, allowed_roots: &[String]) -> Result<Vec<FileEntry>> {
    let roots = canonical_allowed_roots(allowed_roots, false).await?;
    let path = resolve_existing_path(path, &roots).await?;

    #[cfg(unix)]
    {
        return list_files_no_symlink_race(&path).await;
    }

    #[cfg(not(unix))]
    {
        if !fs::metadata(&path)
            .await
            .context("Failed to read metadata")?
            .is_dir()
        {
            bail!("Path is not a directory");
        }

        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(path)
            .await
            .context("Failed to read directory")?;

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .context("Failed to read entry")?
        {
            let entry_path = entry.path();
            let metadata = fs::symlink_metadata(&entry_path)
                .await
                .context("Failed to read metadata")?;
            let file_name = entry.file_name().to_string_lossy().to_string();

            let file_type = if metadata.is_symlink() {
                FileType::Symlink
            } else if metadata.is_dir() {
                FileType::Dir
            } else {
                FileType::File
            };

            let symlink_target = if metadata.is_symlink() {
                fs::read_link(&entry_path)
                    .await
                    .ok()
                    .and_then(|p| p.to_str().map(String::from))
            } else {
                None
            };

            let mode = metadata_mode(&metadata);

            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            entries.push(FileEntry {
                name: file_name,
                file_type,
                size: metadata.len(),
                mode,
                modified_at,
                symlink_target,
            });
        }

        // Sort by name
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(entries)
    }
}

/// Read file content
#[allow(dead_code)]
pub async fn read_file(
    path: &str,
    offset: u64,
    length: u64,
    allowed_roots: &[String],
) -> Result<Vec<u8>> {
    let roots = canonical_allowed_roots(allowed_roots, false).await?;
    let path = resolve_existing_path(path, &roots).await?;

    #[cfg(unix)]
    {
        return read_file_no_symlink_race(&path, offset, length).await;
    }

    #[cfg(not(unix))]
    {
        if !fs::metadata(&path)
            .await
            .context("Failed to read metadata")?
            .is_file()
        {
            bail!("Path is not a file");
        }

        let mut file = fs::File::open(path).await.context("Failed to open file")?;

        if offset > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .context("Failed to seek")?;
        }

        let mut buffer = Vec::new();
        let max_read = if length > 0 {
            length as usize
        } else {
            usize::MAX
        };

        use tokio::io::AsyncReadExt;
        file.take(max_read as u64)
            .read_to_end(&mut buffer)
            .await
            .context("Failed to read file")?;

        Ok(buffer)
    }
}

/// Write file content
#[allow(dead_code)]
pub async fn write_file(
    path: &str,
    data: &[u8],
    mode: Option<u32>,
    create_dirs: bool,
    allowed_roots: &[String],
) -> Result<u64> {
    let roots = canonical_allowed_roots(allowed_roots, true).await?;
    let path = validate_absolute_path(path)?;
    let mut write_target = resolve_write_target(path, &roots).await?;

    if create_dirs {
        if let Some(parent) = write_target.parent() {
            create_dir_all_no_symlink(parent)
                .await
                .context("Failed to create parent directories")?;
        }
    }
    write_target = resolve_write_target(path, &roots).await?;

    let mut file = create_file_no_symlink(&write_target)
        .await
        .context("Failed to create file")?;

    use tokio::io::AsyncWriteExt;
    file.write_all(data).await.context("Failed to write file")?;

    file.flush().await.context("Failed to flush file")?;

    if let Some(mode) = mode {
        set_file_mode(&file, &write_target, mode).await?;
    }

    Ok(data.len() as u64)
}

/// Delete file or directory
#[allow(dead_code)]
pub async fn delete_path(path: &str, recursive: bool, allowed_roots: &[String]) -> Result<()> {
    let roots = canonical_allowed_roots(allowed_roots, false).await?;
    let raw_path = validate_absolute_path(path)?;

    // Safety: prevent deleting root
    if raw_path.parent().is_none() || raw_path.to_str() == Some("/") {
        bail!("Cannot delete root directory");
    }

    #[cfg(unix)]
    {
        return delete_path_no_symlink_race(raw_path, recursive, &roots).await;
    }

    #[cfg(not(unix))]
    {
        let parent = raw_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Path has no parent"))?;
        let canonical_parent = fs::canonicalize(parent)
            .await
            .with_context(|| format!("Parent {} does not exist", parent.display()))?;
        ensure_under_allowed_roots(&canonical_parent, &roots)?;
        let canonical = resolve_existing_path(path, &roots).await?;
        let metadata = fs::symlink_metadata(raw_path)
            .await
            .context("Failed to read metadata")?;
        if metadata.is_symlink() {
            fs::remove_file(raw_path)
                .await
                .context("Failed to delete symlink")?;
            return Ok(());
        }

        if fs::metadata(&canonical)
            .await
            .context("Failed to read metadata")?
            .is_dir()
        {
            if recursive {
                fs::remove_dir_all(raw_path)
                    .await
                    .context("Failed to delete directory")?;
            } else {
                fs::remove_dir(raw_path)
                    .await
                    .context("Failed to delete directory (not empty?)")?;
            }
        } else {
            fs::remove_file(raw_path)
                .await
                .context("Failed to delete file")?;
        }

        Ok(())
    }
}

async fn canonical_allowed_roots(
    allowed_roots: &[String],
    create_missing: bool,
) -> Result<Vec<PathBuf>> {
    if allowed_roots.is_empty() {
        bail!("no allowed file roots configured");
    }
    let mut roots = Vec::with_capacity(allowed_roots.len());
    for root in allowed_roots {
        let root = validate_absolute_path(root)?;
        if create_missing {
            fs::create_dir_all(root)
                .await
                .with_context(|| format!("Failed to create allowed root {}", root.display()))?;
        }
        let canonical = fs::canonicalize(root)
            .await
            .with_context(|| format!("Allowed root {} does not exist", root.display()))?;
        roots.push(canonical);
    }
    Ok(roots)
}

fn validate_absolute_path(path: &str) -> Result<&Path> {
    if path.contains('\0') {
        bail!("Path contains NUL byte");
    }
    let path = Path::new(path.trim());
    if !path.is_absolute() {
        bail!("Path must be absolute");
    }
    Ok(path)
}

async fn resolve_existing_path(path: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
    let path = validate_absolute_path(path)?;
    let canonical = fs::canonicalize(path)
        .await
        .with_context(|| format!("Path {} does not exist", path.display()))?;
    ensure_under_allowed_roots(&canonical, allowed_roots)?;
    Ok(canonical)
}

async fn resolve_write_target(path: &Path, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
    if fs::try_exists(path).await.unwrap_or(false) {
        let canonical = fs::canonicalize(path)
            .await
            .with_context(|| format!("Path {} does not exist", path.display()))?;
        ensure_under_allowed_roots(&canonical, allowed_roots)?;
        return Ok(canonical);
    }
    let mut ancestor = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Path has no parent"))?;
    while !fs::try_exists(ancestor).await.unwrap_or(false) {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Path has no existing parent"))?;
    }
    let raw_ancestor = ancestor;
    let canonical_ancestor = fs::canonicalize(raw_ancestor)
        .await
        .with_context(|| format!("Parent {} does not exist", raw_ancestor.display()))?;
    ensure_under_allowed_roots(&canonical_ancestor, allowed_roots)?;
    let relative = path.strip_prefix(raw_ancestor).with_context(|| {
        format!(
            "Path {} is not below ancestor {}",
            path.display(),
            raw_ancestor.display()
        )
    })?;
    let canonical_target = canonical_ancestor.join(relative);
    if allowed_roots
        .iter()
        .any(|root| canonical_target.starts_with(root))
    {
        Ok(canonical_target)
    } else {
        bail!("Path is outside configured allowed file roots")
    }
}

fn ensure_under_allowed_roots(path: &Path, allowed_roots: &[PathBuf]) -> Result<()> {
    if allowed_roots.iter().any(|root| path.starts_with(root)) {
        Ok(())
    } else {
        bail!("Path is outside configured allowed file roots")
    }
}

#[cfg(unix)]
async fn list_files_no_symlink_race(path: &Path) -> Result<Vec<FileEntry>> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || list_files_no_symlink_race_blocking(&path))
        .await
        .context("Failed to join file list task")?
}

#[cfg(unix)]
async fn read_file_no_symlink_race(path: &Path, offset: u64, length: u64) -> Result<Vec<u8>> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || read_file_no_symlink_race_blocking(&path, offset, length))
        .await
        .context("Failed to join file read task")?
}

#[cfg(unix)]
async fn delete_path_no_symlink_race(
    raw_path: &Path,
    recursive: bool,
    allowed_roots: &[PathBuf],
) -> Result<()> {
    let parent = raw_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Path has no parent"))?;
    let file_name = raw_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Path has no file name"))?
        .to_os_string();
    let canonical_parent = fs::canonicalize(parent)
        .await
        .with_context(|| format!("Parent {} does not exist", parent.display()))?;
    ensure_under_allowed_roots(&canonical_parent, allowed_roots)?;
    let stable_path = canonical_parent.join(file_name);

    tokio::task::spawn_blocking(move || {
        delete_path_no_symlink_race_blocking(&stable_path, recursive)
    })
    .await
    .context("Failed to join file delete task")?
}

#[cfg(unix)]
async fn create_file_no_symlink(path: &Path) -> Result<fs::File> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || create_file_no_symlink_blocking(&path))
        .await
        .context("Failed to join file create task")?
}

#[cfg(unix)]
async fn create_dir_all_no_symlink(path: &Path) -> Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || create_dir_all_no_symlink_blocking(&path))
        .await
        .context("Failed to join directory create task")?
}

#[cfg(unix)]
fn create_dir_all_no_symlink_blocking(path: &Path) -> Result<()> {
    let parts = absolute_normal_components(path)?;
    let mut dir_fd = open_root_dir_no_follow()?;
    for part in parts {
        let parent_fd = dir_fd.as_raw_fd();
        match try_open_dir_at(parent_fd, &part) {
            Ok(next_fd) => dir_fd = next_fd,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                mkdir_child_dir(parent_fd, &part)?;
                dir_fd = open_child_dir_no_follow(parent_fd, &part)?;
            }
            Err(err) => return Err(err).context("Failed to open parent directory"),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn absolute_normal_components(path: &Path) -> Result<Vec<std::ffi::OsString>> {
    let mut components = path.components();
    match components.next() {
        Some(std::path::Component::RootDir) => {}
        _ => bail!("Path must be absolute"),
    }
    components
        .map(|component| match component {
            std::path::Component::Normal(part) => Ok(part.to_os_string()),
            _ => bail!("Path must not contain relative components"),
        })
        .collect()
}

#[cfg(unix)]
fn create_file_no_symlink_blocking(path: &Path) -> Result<fs::File> {
    let parts = absolute_normal_components(path)?;
    let (file_name, parent_parts) = parts
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("Path has no file name"))?;

    let root_fd = open_root_dir_no_follow()?;
    let mut dir_fd = root_fd;
    for part in parent_parts {
        let next_fd = open_child_dir_no_follow(dir_fd.as_raw_fd(), part)?;
        dir_fd = next_fd;
    }
    let file_fd = open_child_file_no_follow(dir_fd.as_raw_fd(), file_name)?;
    let std_file = std::fs::File::from(file_fd);
    Ok(fs::File::from_std(std_file))
}

#[cfg(unix)]
fn list_files_no_symlink_race_blocking(path: &Path) -> Result<Vec<FileEntry>> {
    let dir_fd = open_dir_path_no_follow(path)?;
    let entry_names = read_dir_names(dir_fd.as_raw_fd()).context("Failed to read directory")?;
    let mut entries = Vec::with_capacity(entry_names.len());
    for entry_name in entry_names {
        let metadata = child_metadata_no_follow(dir_fd.as_raw_fd(), &entry_name)
            .context("Failed to read metadata")?;
        let file_type = if is_symlink_mode(metadata.st_mode) {
            FileType::Symlink
        } else if is_dir_mode(metadata.st_mode) {
            FileType::Dir
        } else {
            FileType::File
        };
        let symlink_target = if matches!(file_type, FileType::Symlink) {
            read_link_child(dir_fd.as_raw_fd(), &entry_name, metadata.st_size).ok()
        } else {
            None
        };
        entries.push(FileEntry {
            name: entry_name.to_string_lossy().to_string(),
            file_type,
            size: u64::try_from(metadata.st_size).unwrap_or(0),
            mode: metadata.st_mode as u32,
            modified_at: metadata.st_mtime as i64,
            symlink_target,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

#[cfg(unix)]
fn read_file_no_symlink_race_blocking(path: &Path, offset: u64, length: u64) -> Result<Vec<u8>> {
    let parts = absolute_normal_components(path)?;
    let (file_name, parent_parts) = parts
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("Path has no file name"))?;

    let parent_fd = open_dir_parts_no_follow(parent_parts)?;
    let file_fd = open_child_file_read_no_follow(parent_fd.as_raw_fd(), file_name)?;
    let metadata = file_metadata(file_fd.as_raw_fd()).context("Failed to read metadata")?;
    if !is_file_mode(metadata.st_mode) {
        bail!("Path is not a file");
    }

    let mut file = std::fs::File::from(file_fd);
    if offset > 0 {
        std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(offset))
            .context("Failed to seek")?;
    }

    let mut buffer = Vec::new();
    let max_read = if length > 0 { length } else { u64::MAX };
    std::io::Read::read_to_end(&mut std::io::Read::take(file, max_read), &mut buffer)
        .context("Failed to read file")?;
    Ok(buffer)
}

#[cfg(unix)]
fn delete_path_no_symlink_race_blocking(path: &Path, recursive: bool) -> Result<()> {
    let parts = absolute_normal_components(path)?;
    let (target_name, parent_parts) = parts
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("Path has no file name"))?;

    let parent_fd = open_dir_parts_no_follow(parent_parts)?;

    let metadata = child_metadata_no_follow(parent_fd.as_raw_fd(), target_name)
        .context("Failed to read metadata")?;
    if is_dir_mode(metadata.st_mode) {
        if recursive {
            remove_dir_all_at(parent_fd.as_raw_fd(), target_name)
                .context("Failed to delete directory")?;
        } else {
            unlink_child_dir(parent_fd.as_raw_fd(), target_name)
                .context("Failed to delete directory (not empty?)")?;
        }
    } else {
        unlink_child(parent_fd.as_raw_fd(), target_name).context("Failed to delete file")?;
    }
    Ok(())
}

#[cfg(unix)]
fn remove_dir_all_at(parent_fd: RawFd, name: &std::ffi::OsStr) -> Result<()> {
    let dir_fd = open_child_dir_no_follow(parent_fd, name)?;
    let entries = read_dir_names(dir_fd.as_raw_fd()).context("Failed to read directory")?;
    for entry in entries {
        let metadata = child_metadata_no_follow(dir_fd.as_raw_fd(), &entry)
            .context("Failed to read metadata")?;
        if is_dir_mode(metadata.st_mode) {
            remove_dir_all_at(dir_fd.as_raw_fd(), &entry)?;
        } else {
            unlink_child(dir_fd.as_raw_fd(), &entry)?;
        }
    }
    unlink_child_dir(parent_fd, name)
}

#[cfg(unix)]
fn open_dir_path_no_follow(path: &Path) -> Result<OwnedFd> {
    let parts = absolute_normal_components(path)?;
    open_dir_parts_no_follow(&parts)
}

#[cfg(unix)]
fn open_dir_parts_no_follow(parts: &[std::ffi::OsString]) -> Result<OwnedFd> {
    let root_fd = open_root_dir_no_follow()?;
    let mut dir_fd = root_fd;
    for part in parts {
        let next_fd = open_child_dir_no_follow(dir_fd.as_raw_fd(), part)?;
        dir_fd = next_fd;
    }
    Ok(dir_fd)
}

#[cfg(unix)]
fn open_root_dir_no_follow() -> Result<OwnedFd> {
    open_dir_at(libc::AT_FDCWD, std::ffi::OsStr::new("/"))
}

#[cfg(unix)]
fn open_child_dir_no_follow(
    parent_fd: std::os::unix::io::RawFd,
    name: &std::ffi::OsStr,
) -> Result<OwnedFd> {
    open_dir_at(parent_fd, name)
}

#[cfg(unix)]
fn open_dir_at(parent_fd: std::os::unix::io::RawFd, name: &std::ffi::OsStr) -> Result<OwnedFd> {
    try_open_dir_at(parent_fd, name).context("Failed to open parent directory")
}

#[cfg(unix)]
fn try_open_dir_at(
    parent_fd: std::os::unix::io::RawFd,
    name: &std::ffi::OsStr,
) -> std::io::Result<OwnedFd> {
    let name = os_str_to_cstring(name)?;
    let fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_child_file_no_follow(
    parent_fd: std::os::unix::io::RawFd,
    name: &std::ffi::OsStr,
) -> Result<OwnedFd> {
    let name = os_str_to_cstring(name)?;
    let fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o666,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to open file");
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_child_file_read_no_follow(
    parent_fd: std::os::unix::io::RawFd,
    name: &std::ffi::OsStr,
) -> Result<OwnedFd> {
    let name = os_str_to_cstring(name)?;
    let fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to open file");
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(unix)]
fn mkdir_child_dir(parent_fd: std::os::unix::io::RawFd, name: &std::ffi::OsStr) -> Result<()> {
    let name = os_str_to_cstring(name)?;
    let result = unsafe { libc::mkdirat(parent_fd, name.as_ptr(), 0o777) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to create directory");
    }
    Ok(())
}

#[cfg(unix)]
fn file_metadata(fd: RawFd) -> Result<libc::stat> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe { libc::fstat(fd, metadata.as_mut_ptr()) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to stat file");
    }
    Ok(unsafe { metadata.assume_init() })
}

#[cfg(unix)]
fn child_metadata_no_follow(parent_fd: RawFd, name: &std::ffi::OsStr) -> Result<libc::stat> {
    let name = os_str_to_cstring(name)?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            parent_fd,
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to stat path");
    }
    Ok(unsafe { metadata.assume_init() })
}

#[cfg(unix)]
fn is_dir_mode(mode: libc::mode_t) -> bool {
    (mode & libc::S_IFMT) == libc::S_IFDIR
}

#[cfg(unix)]
fn is_file_mode(mode: libc::mode_t) -> bool {
    (mode & libc::S_IFMT) == libc::S_IFREG
}

#[cfg(unix)]
fn is_symlink_mode(mode: libc::mode_t) -> bool {
    (mode & libc::S_IFMT) == libc::S_IFLNK
}

#[cfg(unix)]
fn unlink_child(parent_fd: RawFd, name: &std::ffi::OsStr) -> Result<()> {
    unlink_child_with_flags(parent_fd, name, 0)
}

#[cfg(unix)]
fn unlink_child_dir(parent_fd: RawFd, name: &std::ffi::OsStr) -> Result<()> {
    unlink_child_with_flags(parent_fd, name, libc::AT_REMOVEDIR)
}

#[cfg(unix)]
fn unlink_child_with_flags(
    parent_fd: RawFd,
    name: &std::ffi::OsStr,
    flags: libc::c_int,
) -> Result<()> {
    let name = os_str_to_cstring(name)?;
    let result = unsafe { libc::unlinkat(parent_fd, name.as_ptr(), flags) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to unlink path");
    }
    Ok(())
}

#[cfg(unix)]
fn read_link_child(
    parent_fd: RawFd,
    name: &std::ffi::OsStr,
    expected_size: libc::off_t,
) -> Result<String> {
    let name = os_str_to_cstring(name)?;
    let buffer_len = usize::try_from(expected_size)
        .unwrap_or(0)
        .saturating_add(1)
        .clamp(256, 65_536);
    let mut buffer = vec![0u8; buffer_len];
    let len = unsafe {
        libc::readlinkat(
            parent_fd,
            name.as_ptr(),
            buffer.as_mut_ptr().cast::<libc::c_char>(),
            buffer.len(),
        )
    };
    if len < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to read symlink");
    }
    let len = usize::try_from(len).unwrap_or(0);
    String::from_utf8(buffer[..len].to_vec()).context("Symlink target is not valid UTF-8")
}

#[cfg(unix)]
fn read_dir_names(fd: RawFd) -> Result<Vec<std::ffi::OsString>> {
    use std::os::unix::ffi::OsStringExt;

    let duplicated_fd = unsafe { libc::dup(fd) };
    if duplicated_fd < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to duplicate directory fd");
    }
    let dir = unsafe { libc::fdopendir(duplicated_fd) };
    if dir.is_null() {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::close(duplicated_fd);
        }
        return Err(err).context("Failed to open directory stream");
    }

    let mut entries = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(dir) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
        let bytes = name.to_bytes();
        if bytes != b"." && bytes != b".." {
            entries.push(std::ffi::OsString::from_vec(bytes.to_vec()));
        }
    }

    let close_result = unsafe { libc::closedir(dir) };
    if close_result < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to close directory stream");
    }
    Ok(entries)
}

#[cfg(unix)]
fn os_str_to_cstring(value: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(value.as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "Path contains NUL byte")
    })
}

#[cfg(not(unix))]
async fn create_file_no_symlink(path: &Path) -> Result<fs::File> {
    fs::File::create(path)
        .await
        .context("Failed to create file")
}

#[cfg(not(unix))]
async fn create_dir_all_no_symlink(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .await
        .context("Failed to create parent directories")
}

#[cfg(unix)]
fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn metadata_mode(_metadata: &std::fs::Metadata) -> u32 {
    0o644
}

#[cfg(unix)]
async fn set_file_mode(file: &fs::File, _path: &Path, mode: u32) -> Result<()> {
    let fd = file.as_raw_fd();
    let result = unsafe { libc::fchmod(fd, mode as libc::mode_t) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("Failed to set permissions");
    }
    Ok(())
}

#[cfg(not(unix))]
async fn set_file_mode(_file: &fs::File, _path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_list_files() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_str().unwrap();
        let roots = vec![temp_path.to_string()];

        // Create test files
        fs::write(temp_dir.path().join("file1.txt"), b"content1")
            .await
            .unwrap();
        fs::write(temp_dir.path().join("file2.txt"), b"content2")
            .await
            .unwrap();
        fs::create_dir(temp_dir.path().join("subdir"))
            .await
            .unwrap();

        let entries = list_files(temp_path, &roots).await.unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn test_read_write_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();
        let roots = vec![temp_dir.path().to_str().unwrap().to_string()];

        let data = b"Hello, World!";
        let written = write_file(file_path_str, data, Some(0o644), false, &roots)
            .await
            .unwrap();
        assert_eq!(written, data.len() as u64);

        let read_data = read_file(file_path_str, 0, 0, &roots).await.unwrap();
        assert_eq!(read_data, data);

        // Test partial read
        let partial = read_file(file_path_str, 7, 5, &roots).await.unwrap();
        assert_eq!(partial, b"World");
    }

    #[tokio::test]
    async fn test_delete_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("delete_me.txt");
        let file_path_str = file_path.to_str().unwrap();
        let roots = vec![temp_dir.path().to_str().unwrap().to_string()];

        fs::write(&file_path, b"delete me").await.unwrap();
        assert!(file_path.exists());

        delete_path(file_path_str, false, &roots).await.unwrap();
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn test_delete_directory_recursive() {
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path().join("test_dir");
        let dir_path_str = dir_path.to_str().unwrap();
        let roots = vec![temp_dir.path().to_str().unwrap().to_string()];

        fs::create_dir(&dir_path).await.unwrap();
        fs::write(dir_path.join("file.txt"), b"content")
            .await
            .unwrap();

        delete_path(dir_path_str, true, &roots).await.unwrap();
        assert!(!dir_path.exists());
    }

    #[tokio::test]
    async fn test_reject_relative_path() {
        let temp_dir = TempDir::new().unwrap();
        let roots = vec![temp_dir.path().to_str().unwrap().to_string()];
        let result = list_files("relative/path", &roots).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("absolute"));
    }

    #[tokio::test]
    async fn test_reject_root_deletion() {
        let temp_dir = TempDir::new().unwrap();
        let roots = vec![temp_dir.path().to_str().unwrap().to_string()];
        let result = delete_path("/", true, &roots).await;
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(message.contains("root") || message.contains("outside"));
    }

    #[tokio::test]
    async fn test_reject_path_outside_allowed_roots() {
        let allowed = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, b"secret").await.unwrap();
        let roots = vec![allowed.path().to_str().unwrap().to_string()];

        let result = read_file(outside_file.to_str().unwrap(), 0, 0, &roots).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("outside"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_write_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let allowed = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let link_path = allowed.path().join("link");
        symlink(outside.path(), &link_path).unwrap();

        let escaped_path = link_path.join("owned.txt");
        let roots = vec![allowed.path().to_str().unwrap().to_string()];
        let result = write_file(
            escaped_path.to_str().unwrap(),
            b"should not escape",
            None,
            false,
            &roots,
        )
        .await;

        assert!(result.is_err());
        assert!(!outside.path().join("owned.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_delete_rejects_replaced_parent_symlink_escape() {
        use std::os::unix::fs::symlink;

        let allowed = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let parent = allowed.path().join("parent");
        let target = parent.join("victim.txt");
        let outside_target = outside.path().join("victim.txt");

        fs::create_dir(&parent).await.unwrap();
        fs::write(&target, b"inside").await.unwrap();
        fs::write(&outside_target, b"outside").await.unwrap();

        let stable_parent = fs::canonicalize(&parent).await.unwrap();
        let stable_target = stable_parent.join("victim.txt");
        fs::remove_dir_all(&parent).await.unwrap();
        symlink(outside.path(), &parent).unwrap();

        let result = tokio::task::spawn_blocking(move || {
            delete_path_no_symlink_race_blocking(&stable_target, false)
        })
        .await
        .unwrap();

        assert!(result.is_err());
        assert!(outside_target.exists());
        assert_eq!(std::fs::read(&outside_target).unwrap(), b"outside");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_read_rejects_replaced_parent_symlink_escape() {
        use std::os::unix::fs::symlink;

        let allowed = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let parent = allowed.path().join("parent");
        let inside_target = parent.join("secret.txt");
        let outside_target = outside.path().join("secret.txt");

        fs::create_dir(&parent).await.unwrap();
        fs::write(&inside_target, b"inside").await.unwrap();
        fs::write(&outside_target, b"outside-secret").await.unwrap();

        let stable_parent = fs::canonicalize(&parent).await.unwrap();
        let stable_target = stable_parent.join("secret.txt");
        fs::remove_dir_all(&parent).await.unwrap();
        symlink(outside.path(), &parent).unwrap();

        let result = tokio::task::spawn_blocking(move || {
            read_file_no_symlink_race_blocking(&stable_target, 0, 0)
        })
        .await
        .unwrap();

        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_list_rejects_replaced_parent_symlink_escape() {
        use std::os::unix::fs::symlink;

        let allowed = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let list_dir = allowed.path().join("list-me");
        let outside_list_dir = outside.path().join("list-me");

        fs::create_dir(&list_dir).await.unwrap();
        fs::write(list_dir.join("inside.txt"), b"inside")
            .await
            .unwrap();
        fs::create_dir(&outside_list_dir).await.unwrap();
        fs::write(outside_list_dir.join("outside-secret.txt"), b"outside")
            .await
            .unwrap();

        let stable_list_dir = fs::canonicalize(&list_dir).await.unwrap();
        fs::remove_dir_all(&list_dir).await.unwrap();
        symlink(outside.path(), &list_dir).unwrap();

        let result = tokio::task::spawn_blocking(move || {
            list_files_no_symlink_race_blocking(&stable_list_dir)
        })
        .await
        .unwrap();

        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_recursive_delete_unlinks_nested_symlink_without_following() {
        use std::os::unix::fs::symlink;

        let allowed = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let dir_path = allowed.path().join("tree");
        let nested = dir_path.join("nested");
        let link_path = dir_path.join("outside-link");
        let outside_file = outside.path().join("secret.txt");
        let roots = vec![allowed.path().to_str().unwrap().to_string()];

        fs::create_dir(&dir_path).await.unwrap();
        fs::create_dir(&nested).await.unwrap();
        fs::write(nested.join("file.txt"), b"inside").await.unwrap();
        fs::write(&outside_file, b"secret").await.unwrap();
        symlink(outside.path(), &link_path).unwrap();

        delete_path(dir_path.to_str().unwrap(), true, &roots)
            .await
            .unwrap();

        assert!(!dir_path.exists());
        assert!(outside_file.exists());
        assert_eq!(std::fs::read(&outside_file).unwrap(), b"secret");
    }
}
