#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{bail, Context, Result};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
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
pub async fn list_files(path: &str) -> Result<Vec<FileEntry>> {
    let path = Path::new(path);

    if !path.is_absolute() {
        bail!("Path must be absolute");
    }

    if !path.exists() {
        bail!("Path does not exist");
    }

    if !path.is_dir() {
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
        let metadata = entry.metadata().await.context("Failed to read metadata")?;
        let file_name = entry.file_name().to_string_lossy().to_string();

        let file_type = if metadata.is_symlink() {
            FileType::Symlink
        } else if metadata.is_dir() {
            FileType::Dir
        } else {
            FileType::File
        };

        let symlink_target = if metadata.is_symlink() {
            fs::read_link(entry.path())
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

/// Read file content
#[allow(dead_code)]
pub async fn read_file(path: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
    let path = Path::new(path);

    if !path.is_absolute() {
        bail!("Path must be absolute");
    }

    if !path.exists() {
        bail!("File does not exist");
    }

    if !path.is_file() {
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

/// Write file content
#[allow(dead_code)]
pub async fn write_file(
    path: &str,
    data: &[u8],
    mode: Option<u32>,
    create_dirs: bool,
) -> Result<u64> {
    let path = Path::new(path);

    if !path.is_absolute() {
        bail!("Path must be absolute");
    }

    if create_dirs {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .context("Failed to create parent directories")?;
        }
    }

    let mut file = fs::File::create(path)
        .await
        .context("Failed to create file")?;

    use tokio::io::AsyncWriteExt;
    file.write_all(data).await.context("Failed to write file")?;

    file.flush().await.context("Failed to flush file")?;

    if let Some(mode) = mode {
        set_file_mode(path, mode).await?;
    }

    Ok(data.len() as u64)
}

/// Delete file or directory
#[allow(dead_code)]
pub async fn delete_path(path: &str, recursive: bool) -> Result<()> {
    let path = Path::new(path);

    if !path.is_absolute() {
        bail!("Path must be absolute");
    }

    // Safety: prevent deleting root
    if path.parent().is_none() || path.to_str() == Some("/") {
        bail!("Cannot delete root directory");
    }

    if !path.exists() {
        bail!("Path does not exist");
    }

    if path.is_dir() {
        if recursive {
            fs::remove_dir_all(path)
                .await
                .context("Failed to delete directory")?;
        } else {
            fs::remove_dir(path)
                .await
                .context("Failed to delete directory (not empty?)")?;
        }
    } else {
        fs::remove_file(path)
            .await
            .context("Failed to delete file")?;
    }

    Ok(())
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
async fn set_file_mode(path: &Path, mode: u32) -> Result<()> {
    let permissions = std::fs::Permissions::from_mode(mode);
    fs::set_permissions(path, permissions)
        .await
        .context("Failed to set permissions")
}

#[cfg(not(unix))]
async fn set_file_mode(_path: &Path, _mode: u32) -> Result<()> {
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

        let entries = list_files(temp_path).await.unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn test_read_write_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_path_str = file_path.to_str().unwrap();

        let data = b"Hello, World!";
        let written = write_file(file_path_str, data, Some(0o644), false)
            .await
            .unwrap();
        assert_eq!(written, data.len() as u64);

        let read_data = read_file(file_path_str, 0, 0).await.unwrap();
        assert_eq!(read_data, data);

        // Test partial read
        let partial = read_file(file_path_str, 7, 5).await.unwrap();
        assert_eq!(partial, b"World");
    }

    #[tokio::test]
    async fn test_delete_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("delete_me.txt");
        let file_path_str = file_path.to_str().unwrap();

        fs::write(&file_path, b"delete me").await.unwrap();
        assert!(file_path.exists());

        delete_path(file_path_str, false).await.unwrap();
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn test_delete_directory_recursive() {
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path().join("test_dir");
        let dir_path_str = dir_path.to_str().unwrap();

        fs::create_dir(&dir_path).await.unwrap();
        fs::write(dir_path.join("file.txt"), b"content")
            .await
            .unwrap();

        delete_path(dir_path_str, true).await.unwrap();
        assert!(!dir_path.exists());
    }

    #[tokio::test]
    async fn test_reject_relative_path() {
        let result = list_files("relative/path").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("absolute"));
    }

    #[tokio::test]
    async fn test_reject_root_deletion() {
        let result = delete_path("/", true).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("root"));
    }
}
