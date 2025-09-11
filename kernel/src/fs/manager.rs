use crate::fs::disk::AtaDisk;
use crate::fs::fat32::{Fat32FileSystem, FileEntry};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::instructions::interrupts::{self, without_interrupts};

lazy_static! {
    pub static ref FILESYSTEM: Mutex<Option<Fat32FileSystem<AtaDisk>>> = Mutex::new(None);
}

/// Initialize the filesystem
pub fn init_filesystem() -> Result<(), &'static str> {
    crate::serial_println!("Initializing filesystem...");

    // Try primary master first (drive 0)
    crate::serial_println!("Trying primary master drive (0)...");
    let mut disk = AtaDisk::new_primary(0);
    if let Ok(_) = disk.init() {
        crate::serial_println!("Primary master initialized successfully");
        match Fat32FileSystem::new(disk) {
            Ok(filesystem) => {
                crate::serial_println!("FAT32 filesystem found on primary master");
                *FILESYSTEM.lock() = Some(filesystem);
                return Ok(());
            }
            Err(e) => {
                crate::serial_println!("Primary master is not FAT32: {}", e);
            }
        }
    } else {
        crate::serial_println!("Failed to initialize primary master");
    }

    // Try primary slave (drive 1)
    crate::serial_println!("Trying primary slave drive (1)...");
    let mut disk = AtaDisk::new_primary(1);
    if let Ok(_) = disk.init() {
        crate::serial_println!("Primary slave initialized successfully");
        match Fat32FileSystem::new(disk) {
            Ok(filesystem) => {
                crate::serial_println!("FAT32 filesystem found on primary slave");
                *FILESYSTEM.lock() = Some(filesystem);
                return Ok(());
            }
            Err(e) => {
                crate::serial_println!("Primary slave is not FAT32: {}", e);
            }
        }
    } else {
        crate::serial_println!("Failed to initialize primary slave");
    }

    Err("No FAT32 filesystem found on any drive")
}

/// Parse a path into its components and return (directory_cluster, filename)
/// Returns the cluster of the parent directory and the filename
fn resolve_path(path: &str) -> Result<(u32, Option<String>), &'static str> {
    resolve_path_for_operation(path, false)
}

fn resolve_path_for_operation(
    path: &str,
    for_deletion: bool,
) -> Result<(u32, Option<String>), &'static str> {
    let mut fs_guard = FILESYSTEM.lock();
    let fs = match fs_guard.as_mut() {
        Some(fs) => fs,
        None => return Err("Filesystem not initialized"),
    };

    let root_cluster = fs.get_root_cluster();

    // Handle empty path or just "/"
    if path.is_empty() || path == "/" {
        return Ok((root_cluster, None));
    }

    // Remove leading slash and split path
    let path = path.strip_prefix('/').unwrap_or(path);
    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();

    if components.is_empty() {
        return Ok((root_cluster, None));
    }

    let mut current_cluster = root_cluster;

    // Navigate through all components except the last one
    for component in &components[..components.len().saturating_sub(1)] {
        current_cluster = fs.navigate_to_directory(current_cluster, component)?;
    }

    // For deletion operations, we don't navigate into the target directory
    // We want to return the parent directory and the directory name to delete
    if for_deletion {
        let filename = components.last().map(|s| s.to_string());
        return Ok((current_cluster, filename));
    }

    // Check if the last component is a directory
    if let Some(last) = components.last() {
        if fs.is_directory(current_cluster, last)? {
            current_cluster = fs.navigate_to_directory(current_cluster, last)?;

            return Ok((current_cluster, None));
        }
    }

    // Return the parent directory cluster and the filename
    let filename = components.last().map(|s| s.to_string());

    Ok((current_cluster, filename))
}

/// List files in a directory (path-based)
pub fn list_directory(path: &str) -> Result<Vec<FileEntry>, &'static str> {
    let (dir_cluster, filename) = resolve_path(path)?;

    // If filename is provided, we're looking for a specific file
    if let Some(_) = filename {
        return Err("Path points to a file, not a directory");
    }

    let mut fs_guard = FILESYSTEM.lock();
    match fs_guard.as_mut() {
        Some(fs) => {
            if fs.is_root_directory(dir_cluster) {
                fs.list_root_directory()
            } else {
                fs.list_directory(dir_cluster)
            }
        }
        None => Err("Filesystem not initialized"),
    }
}

/// Find a file by path
pub fn find_file(path: &str) -> Result<Option<FileEntry>, &'static str> {
    let (dir_cluster, filename) = resolve_path(path)?;

    let filename = match filename {
        Some(name) => name,
        None => {
            return Err("Path does not specify a filename");
        }
    };

    let mut fs_guard = FILESYSTEM.lock();
    match fs_guard.as_mut() {
        Some(fs) => {
            let result = fs.find_file_in_directory(dir_cluster, &filename);
            result
        }
        None => Err("Filesystem not initialized"),
    }
}

/// Read a file's content by path
pub fn read_file(path: &str) -> Result<Vec<u8>, &'static str> {
    let file_entry = find_file(path)?.ok_or("File not found")?;

    if file_entry.is_directory {
        return Err("Path points to a directory, not a file");
    }

    let (dir_cluster, filename) = resolve_path(path)?;
    let filename = filename.ok_or("Invalid file path")?;

    let result = {
        let mut fs_guard = FILESYSTEM.lock();
        match fs_guard.as_mut() {
            Some(fs) => fs.read_file(file_entry.first_cluster, file_entry.size),
            None => return Err("Filesystem not initialized"),
        }
    };

    // Update last access time after successful read
    if result.is_ok() {
        without_interrupts(|| {
            let mut fs_guard = FILESYSTEM.lock();
            if let Some(fs) = fs_guard.as_mut() {
                // We ignore the error here since read operation was successful
                // but log if there's an issue updating access time
                if let Err(_) = fs.update_file_last_access(dir_cluster, &filename) {
                    crate::serial_println!(
                        "Warning: Failed to update last access time for file: {}",
                        path
                    );
                }
            }
        });
    }

    result
}

/// Read a text file and return it as a string
pub fn read_text_file(path: &str) -> Result<String, &'static str> {
    let data = read_file(path)?;
    match String::from_utf8(data) {
        Ok(text) => Ok(text),
        Err(_) => Err("File is not valid UTF-8"),
    }
}

/// Create a new file with path-based addressing
pub fn create_file(path: &str, data: &[u8]) -> Result<(), &'static str> {
    let (dir_cluster, filename) = resolve_path(path)?;

    let filename = match filename {
        Some(name) => name,
        None => return Err("Path does not specify a filename"),
    };

    interrupts::without_interrupts(|| {
        let mut fs_guard = FILESYSTEM.lock();
        match fs_guard.as_mut() {
            Some(fs) => fs.create_file(dir_cluster, &filename, data),
            None => Err("Filesystem not initialized"),
        }
    })
}

/// Create a text file with path-based addressing
pub fn create_text_file(path: &str, content: &str) -> Result<(), &'static str> {
    create_file(path, content.as_bytes())
}

/// Delete a file by path
pub fn delete_file(path: &str) -> Result<(), &'static str> {
    let (dir_cluster, filename) = resolve_path(path)?;

    let filename = match filename {
        Some(name) => name,
        None => return Err("Path does not specify a filename"),
    };

    interrupts::without_interrupts(|| {
        let mut fs_guard = FILESYSTEM.lock();
        match fs_guard.as_mut() {
            Some(fs) => fs.delete_file(dir_cluster, &filename),
            None => Err("Filesystem not initialized"),
        }
    })
}

/// Create a new directory with path-based addressing
pub fn create_directory(path: &str) -> Result<(), &'static str> {
    let (parent_cluster, dirname) = resolve_path(path)?;

    let dirname = match dirname {
        Some(name) => name,
        None => return Err("Path does not specify a directory name"),
    };

    interrupts::without_interrupts(|| {
        let mut fs_guard = FILESYSTEM.lock();
        match fs_guard.as_mut() {
            Some(fs) => fs.create_directory(parent_cluster, &dirname),
            None => Err("Filesystem not initialized"),
        }
    })
}

/// Delete a directory by path
pub fn delete_directory(path: &str) -> Result<(), &'static str> {
    let (parent_cluster, dirname) = resolve_path_for_operation(path, true)?;

    let dirname = match dirname {
        Some(name) => name,
        None => return Err("Cannot delete root directory"),
    };

    interrupts::without_interrupts(|| {
        let mut fs_guard = FILESYSTEM.lock();
        match fs_guard.as_mut() {
            Some(fs) => fs.delete_directory(parent_cluster, &dirname),
            None => Err("Filesystem not initialized"),
        }
    })
}

/// Write data to an existing file by path
pub fn write_file(path: &str, data: &[u8]) -> Result<(), &'static str> {
    if let Some(file_entry) = find_file(path)? {
        if file_entry.is_directory {
            return Err("Path points to a directory, not a file");
        }

        // Use the new update_file method which handles cluster allocation/deallocation properly
        let (dir_cluster, filename) = resolve_path(path)?;
        let filename = filename.ok_or("Invalid file path")?;

        interrupts::without_interrupts(|| {
            let mut fs_guard = FILESYSTEM.lock();
            match fs_guard.as_mut() {
                Some(fs) => {
                    let result = fs.update_file(dir_cluster, &filename, data);

                    result
                }
                None => Err("Filesystem not initialized"),
            }
        })
    } else {
        // File doesn't exist, create it
        create_file(path, data)
    }
}

/// Check if a path exists and return whether it's a file or directory
pub fn path_exists(path: &str) -> Result<Option<bool>, &'static str> {
    // Handle root directory specially
    if path == "/" || path.is_empty() {
        return Ok(Some(true)); // true = directory
    }

    let (dir_cluster, filename) = resolve_path(path)?;

    let filename = match filename {
        Some(name) => name,
        None => return Ok(Some(true)), // Directory path without filename
    };

    let mut fs_guard = FILESYSTEM.lock();
    match fs_guard.as_mut() {
        Some(fs) => {
            match fs.find_file_in_directory(dir_cluster, &filename)? {
                Some(entry) => Ok(Some(entry.is_directory)),
                None => Ok(None), // Path doesn't exist
            }
        }
        None => Err("Filesystem not initialized"),
    }
}

pub fn is_file(path: &str) -> Result<bool, &'static str> {
    let (dir_cluster, filename) = resolve_path(path)?;

    if let Some(filename) = filename {
        without_interrupts(|| {
            let mut fs_guard = FILESYSTEM.lock();
            match fs_guard.as_mut() {
                Some(fs) => fs.is_file(dir_cluster, &filename),
                None => Err("Filesystem not initialized"),
            }
        })
    } else {
        Ok(false)
    }
}

/// Copy a file from source path to destination path
pub fn copy_file(source_path: &str, dest_path: &str) -> Result<(), &'static str> {
    // Check if source exists and is a file
    let source_entry = find_file(source_path)?.ok_or("Source file not found")?;
    if source_entry.is_directory {
        return Err("Source path points to a directory, not a file");
    }

    // Check if destination already exists
    if let Ok(Some(_)) = path_exists(dest_path) {
        return Err("Destination already exists");
    }

    // Read the source file data
    let data = read_file(source_path)?;

    // Create the destination file
    create_file(dest_path, &data)?;

    Ok(())
}

/// Copy a directory recursively from source path to destination path
pub fn copy_directory(source_path: &str, dest_path: &str) -> Result<(), &'static str> {
    // Check if source exists and is a directory
    match path_exists(source_path)? {
        Some(true) => {} // It's a directory
        Some(false) => return Err("Source path points to a file, not a directory"),
        None => return Err("Source directory not found"),
    }

    // Check if destination already exists
    if let Ok(Some(_)) = path_exists(dest_path) {
        return Err("Destination already exists");
    }

    // Create the destination directory
    create_directory(dest_path)?;

    // List contents of source directory
    let entries = list_directory(source_path)?;

    // Copy each entry
    for entry in entries {
        let source_item_path = if source_path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", source_path, entry.name)
        };

        let dest_item_path = if dest_path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", dest_path, entry.name)
        };

        if entry.is_directory {
            copy_directory(&source_item_path, &dest_item_path)?;
        } else {
            copy_file(&source_item_path, &dest_item_path)?;
        }
    }

    Ok(())
}

/// Move a file or directory from source path to destination path
pub fn move_item(source_path: &str, dest_path: &str) -> Result<(), &'static str> {
    // Check if source exists
    let is_directory = match path_exists(source_path)? {
        Some(is_dir) => is_dir,
        None => return Err("Source path not found"),
    };

    // Check if destination already exists
    if let Ok(Some(_)) = path_exists(dest_path) {
        return Err("Destination already exists");
    }

    // Resolve source and destination paths
    let (source_dir_cluster, source_filename) = resolve_path(source_path)?;
    let source_filename = source_filename.ok_or("Invalid source path")?;

    let (dest_dir_cluster, dest_filename) = resolve_path(dest_path)?;
    let dest_filename = dest_filename.ok_or("Invalid destination path")?;

    // Try the optimized move first (only works within the same filesystem)
    let optimized_result = interrupts::without_interrupts(|| {
        let mut fs_guard = FILESYSTEM.lock();
        match fs_guard.as_mut() {
            Some(fs) => fs.move_entry(
                source_dir_cluster,
                &source_filename,
                dest_dir_cluster,
                &dest_filename,
            ),
            None => Err("Filesystem not initialized"),
        }
    });

    match optimized_result {
        Ok(()) => Ok(()),
        Err(_) => {
            // Fall back to copy+delete if optimized move fails
            crate::serial_println!("Optimized move failed, falling back to copy+delete");

            // Copy the item first
            if is_directory {
                copy_directory(source_path, dest_path)?;
                // Delete the source directory
                delete_directory(source_path)?;
            } else {
                copy_file(source_path, dest_path)?;
                // Delete the source file
                delete_file(source_path)?;
            }
            Ok(())
        }
    }
}

/// Rename a file or directory (move within the same parent directory)
pub fn rename_item(old_path: &str, new_name: &str) -> Result<(), &'static str> {
    // Validate new name doesn't contain path separators
    if new_name.contains('/') {
        return Err("New name cannot contain path separators");
    }

    // Get the parent directory of the old path
    let old_path_trimmed = old_path.trim_end_matches('/');
    let parent_path = if let Some(last_slash) = old_path_trimmed.rfind('/') {
        if last_slash == 0 {
            "/" // Root directory
        } else {
            &old_path_trimmed[..last_slash]
        }
    } else {
        "/" // Item is in root directory
    };

    // Construct the new path
    let new_path = if parent_path == "/" {
        format!("/{}", new_name)
    } else {
        format!("{}/{}", parent_path, new_name)
    };

    // Use move_item to perform the rename
    move_item(old_path, &new_path)
}
