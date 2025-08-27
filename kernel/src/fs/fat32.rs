use crate::serial_println;
use crate::time::{Date, DateTime, Time, get_utc_time};
use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use core::mem;

/// Boot sector of a FAT32 filesystem
#[repr(packed)]
#[derive(Debug, Clone, Copy)]
pub struct Fat32BootSector {
    pub jump_instruction: [u8; 3],
    pub oem_name: [u8; 8],
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub fat_count: u8,
    pub root_dir_entries: u16,
    pub total_sectors_16: u16,
    pub media_descriptor: u8,
    pub sectors_per_fat_16: u16,
    pub sectors_per_track: u16,
    pub head_count: u16,
    pub hidden_sectors: u32,
    pub total_sectors_32: u32,

    // FAT32 specific fields
    pub sectors_per_fat_32: u32,
    pub ext_flags: u16,
    pub filesystem_version: u16,
    pub root_cluster: u32,
    pub filesystem_info: u16,
    pub backup_boot_sector: u16,
    pub reserved: [u8; 12],
    pub drive_number: u8,
    pub reserved1: u8,
    pub boot_signature: u8,
    pub volume_id: u32,
    pub volume_label: [u8; 11],
    pub filesystem_type: [u8; 8],
    pub boot_code: [u8; 420],
    pub bootable_partition_signature: u16,
}

/// Directory entry structure for FAT32
#[repr(packed)]
#[derive(Debug, Clone, Copy)]
pub struct DirectoryEntry {
    pub name: [u8; 11],
    pub attributes: u8,
    pub reserved: u8,
    pub creation_time_tenths: u8,
    pub creation_time: u16,
    pub creation_date: u16,
    pub last_access_date: u16,
    pub first_cluster_high: u16,
    pub last_write_time: u16,
    pub last_write_date: u16,
    pub first_cluster_low: u16,
    pub file_size: u32,
}

/// Long filename directory entry structure
#[repr(packed)]
#[derive(Debug, Clone, Copy)]
pub struct LongFilenameEntry {
    pub sequence: u8,       // Sequence number (0x40 bit set for last entry)
    pub name1: [u16; 5],    // First 5 characters (UTF-16)
    pub attributes: u8,     // Always 0x0F (LONG_NAME)
    pub reserved: u8,       // Always 0
    pub checksum: u8,       // Checksum of 8.3 name
    pub name2: [u16; 6],    // Next 6 characters (UTF-16)
    pub first_cluster: u16, // Always 0
    pub name3: [u16; 2],    // Last 2 characters (UTF-16)
}

/// File attributes
pub mod attributes {
    pub const READ_ONLY: u8 = 0x01;
    pub const HIDDEN: u8 = 0x02;
    pub const SYSTEM: u8 = 0x04;
    pub const VOLUME_ID: u8 = 0x08;
    pub const DIRECTORY: u8 = 0x10;
    pub const ARCHIVE: u8 = 0x20;
    pub const LONG_NAME: u8 = READ_ONLY | HIDDEN | SYSTEM | VOLUME_ID;
}

/// FAT32 cluster values
pub mod cluster_values {
    pub const FREE: u32 = 0x00000000;
    pub const BAD: u32 = 0x0FFFFFF7;
    pub const END_OF_CHAIN: u32 = 0x0FFFFFFF;
    pub const MASK: u32 = 0x0FFFFFFF;
}

/// Represents a file or directory in the FAT32 filesystem
#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: u32,
    pub first_cluster: u32,

    pub created_at: DateTime,
    pub last_access_at: Date,
    pub last_write_at: DateTime,
}

/// Trait for disk operations
pub trait DiskOperations {
    fn read_sector(&mut self, sector: u64, buffer: &mut [u8]) -> Result<(), &'static str>;
    fn write_sector(&mut self, sector: u64, buffer: &[u8]) -> Result<(), &'static str>;
}

/// FAT32 filesystem implementation
pub struct Fat32FileSystem<D: DiskOperations> {
    disk: D,
    boot_sector: Fat32BootSector,
    fat_start_sector: u64,
    data_start_sector: u64,
    sectors_per_cluster: u64,
    bytes_per_sector: u64,
}

impl<D: DiskOperations> Fat32FileSystem<D> {
    /// Create a new FAT32 filesystem instance
    pub fn new(mut disk: D) -> Result<Self, &'static str> {
        let mut boot_sector_data = [0u8; 512];
        disk.read_sector(0, &mut boot_sector_data)?;

        let boot_sector = unsafe { *(boot_sector_data.as_ptr() as *const Fat32BootSector) };

        // Verify this is a FAT32 filesystem
        if boot_sector.bootable_partition_signature != 0xAA55 {
            return Err("Invalid boot sector signature");
        }

        if boot_sector.sectors_per_fat_16 != 0 || boot_sector.sectors_per_fat_32 == 0 {
            return Err("This is not a FAT32 filesystem (FAT16/12 detected)");
        }

        let fat_start_sector = boot_sector.reserved_sectors as u64;
        let fat_size = boot_sector.sectors_per_fat_32 as u64;
        let data_start_sector = fat_start_sector + (boot_sector.fat_count as u64 * fat_size);

        Ok(Fat32FileSystem {
            disk,
            boot_sector,
            fat_start_sector,
            data_start_sector,
            sectors_per_cluster: boot_sector.sectors_per_cluster as u64,
            bytes_per_sector: boot_sector.bytes_per_sector as u64,
        })
    }

    /// Convert raw FAT date to Date struct
    fn raw_date_to_date(&self, raw: u16) -> Date {
        let year = ((raw >> 9) & 0x7F) + 1980;
        let month = ((raw >> 5) & 0x0F) as u8;
        let day = (raw & 0x1F) as u8;
        Date { day, month, year }
    }

    /// Converts raw FAT time to Time struct
    fn raw_time_to_time(&self, raw: u16) -> Time {
        let hours = ((raw >> 11) & 0x1F) as u8;
        let minutes = ((raw >> 5) & 0x3F) as u8;
        let seconds = ((raw & 0x1F) * 2) as u8;
        Time {
            millis: 0,
            seconds,
            minutes,
            hours,
        }
    }

    fn time_to_raw_time(&self, time: Time) -> u16 {
        let hours = (time.hours as u16 & 0x1F) << 11;
        let minutes = (time.minutes as u16 & 0x3F) << 5;
        let seconds = (time.seconds / 2) as u16 & 0x1F;
        hours | minutes | seconds
    }

    fn date_to_raw_date(&self, date: Date) -> u16 {
        let year = ((date.year - 1980) as u16 & 0x7F) << 9;
        let month = (date.month as u16 & 0x0F) << 5;
        let day = date.day as u16 & 0x1F;
        year | month | day
    }

    /// Calculate checksum for 8.3 filename (used by LFN entries)
    fn calculate_checksum(&self, name_8_3: &[u8; 11]) -> u8 {
        let mut sum = 0u8;
        for &byte in name_8_3.iter() {
            sum = ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(byte);
        }
        sum
    }

    /// Convert UTF-8 string to UTF-16 for LFN entries
    fn utf8_to_utf16(&self, input: &str) -> Vec<u16> {
        input.chars().map(|c| c as u16).collect()
    }

    /// Convert UTF-16 to UTF-8 string from LFN entries
    fn utf16_to_utf8(&self, input: &[u16]) -> String {
        let mut result = String::new();
        for &code_unit in input {
            if code_unit == 0 || code_unit == 0xFFFF {
                break; // End of string or padding
            }
            if let Some(ch) = char::from_u32(code_unit as u32) {
                result.push(ch);
            }
        }
        result
    }

    /// Generate a unique 8.3 filename from a long filename
    fn generate_short_name(
        &mut self,
        dir_cluster: u32,
        long_name: &str,
    ) -> Result<[u8; 11], &'static str> {
        let mut name_8_3 = [b' '; 11];

        // Extract base name and extension
        let (base_name, extension) = if let Some(dot_pos) = long_name.rfind('.') {
            (&long_name[..dot_pos], Some(&long_name[dot_pos + 1..]))
        } else {
            (long_name, None)
        };

        // Convert to uppercase and remove invalid characters
        let clean_base: String = base_name
            .chars()
            .filter(|&c| c.is_ascii_alphanumeric() || c == '_')
            .map(|c| c.to_ascii_uppercase())
            .collect();

        let clean_ext: Option<String> = extension.map(|ext| {
            ext.chars()
                .filter(|&c| c.is_ascii_alphanumeric() || c == '_')
                .map(|c| c.to_ascii_uppercase())
                .take(3)
                .collect()
        });

        // Try numeric tail generation (NAME~1, NAME~2, etc.)
        for tail_num in 1..=999999 {
            // Create base name with tail
            let tail_str = format!("~{}", tail_num);
            let max_base_len = 8 - tail_str.len();
            let truncated_base = if clean_base.len() > max_base_len {
                &clean_base[..max_base_len]
            } else {
                &clean_base
            };

            let short_base = format!("{}{}", truncated_base, tail_str);

            // Fill in the name part
            let base_bytes = short_base.as_bytes();
            let base_len = core::cmp::min(base_bytes.len(), 8);
            name_8_3[..base_len].copy_from_slice(&base_bytes[..base_len]);

            // Fill in the extension part
            if let Some(ref ext) = clean_ext {
                let ext_bytes = ext.as_bytes();
                let ext_len = core::cmp::min(ext_bytes.len(), 3);
                name_8_3[8..8 + ext_len].copy_from_slice(&ext_bytes[..ext_len]);
            }

            // Check if this name already exists
            let short_name_str = self.name_8_3_to_string(&name_8_3);
            if self
                .find_file_in_directory(dir_cluster, &short_name_str)?
                .is_none()
            {
                return Ok(name_8_3);
            }
        }

        Err("Could not generate unique short filename")
    }

    /// Convert 8.3 name array to string
    fn name_8_3_to_string(&self, name_8_3: &[u8; 11]) -> String {
        let mut result = String::new();

        // Add base name
        for i in 0..8 {
            if name_8_3[i] != b' ' {
                result.push(name_8_3[i] as char);
            } else {
                break;
            }
        }

        // Add extension if present
        if name_8_3[8] != b' ' {
            result.push('.');
            for i in 8..11 {
                if name_8_3[i] != b' ' {
                    result.push(name_8_3[i] as char);
                } else {
                    break;
                }
            }
        }

        result
    }

    /// Get the sector number for a given cluster
    fn cluster_to_sector(&self, cluster: u32) -> u64 {
        if cluster < 2 {
            return 0; // Invalid cluster
        }
        self.data_start_sector + (cluster as u64 - 2) * self.sectors_per_cluster
    }

    /// Read a cluster from the disk
    fn read_cluster(&mut self, cluster: u32, buffer: &mut [u8]) -> Result<(), &'static str> {
        let sector = self.cluster_to_sector(cluster);
        let cluster_size = self.sectors_per_cluster * self.bytes_per_sector;

        if buffer.len() < cluster_size as usize {
            return Err("Buffer too small for cluster");
        }

        for i in 0..self.sectors_per_cluster {
            let sector_offset = i * self.bytes_per_sector as u64;
            self.disk.read_sector(
                sector + i,
                &mut buffer
                    [sector_offset as usize..(sector_offset + self.bytes_per_sector) as usize],
            )?;
        }

        Ok(())
    }

    /// Read the next cluster from the FAT
    fn get_next_cluster(&mut self, cluster: u32) -> Result<u32, &'static str> {
        let fat_offset = cluster * 4; // 4 bytes per FAT32 entry
        let fat_sector = self.fat_start_sector + (fat_offset as u64 / self.bytes_per_sector);
        let sector_offset = (fat_offset as u64 % self.bytes_per_sector) as usize;

        let mut sector_buffer = [0u8; 512];
        self.disk.read_sector(fat_sector, &mut sector_buffer)?;

        let fat_entry = u32::from_le_bytes([
            sector_buffer[sector_offset],
            sector_buffer[sector_offset + 1],
            sector_buffer[sector_offset + 2],
            sector_buffer[sector_offset + 3],
        ]) & cluster_values::MASK;

        Ok(fat_entry)
    }

    /// Read directory entries from a cluster with long filename support
    fn read_directory_entries(
        &mut self,
        cluster: u32,
    ) -> Result<Vec<DirectoryEntry>, &'static str> {
        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let mut cluster_buffer = vec![0u8; cluster_size];
        let mut entries = Vec::new();
        let mut current_cluster = cluster;

        loop {
            self.read_cluster(current_cluster, &mut cluster_buffer)?;

            let entries_per_cluster = cluster_size / mem::size_of::<DirectoryEntry>();

            for i in 0..entries_per_cluster {
                let entry_offset = i * mem::size_of::<DirectoryEntry>();
                let entry = unsafe {
                    *(cluster_buffer.as_ptr().add(entry_offset) as *const DirectoryEntry)
                };

                // Check if this is the end of directory entries
                if entry.name[0] == 0x00 {
                    return Ok(entries);
                }

                // Skip deleted entries and long filename entries (we'll process them separately)
                if entry.name[0] == 0xE5 || entry.attributes == attributes::LONG_NAME {
                    continue;
                }

                entries.push(entry);
            }

            // Get the next cluster in the chain
            let next_cluster = self.get_next_cluster(current_cluster)?;
            if next_cluster >= cluster_values::END_OF_CHAIN {
                break;
            }
            current_cluster = next_cluster;
        }

        Ok(entries)
    }

    /// Read directory entries with long filename support
    fn read_directory_entries_with_lfn(
        &mut self,
        cluster: u32,
    ) -> Result<Vec<(Option<String>, DirectoryEntry)>, &'static str> {
        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let mut cluster_buffer = vec![0u8; cluster_size];
        let mut entries = Vec::new();
        let mut current_cluster = cluster;

        loop {
            self.read_cluster(current_cluster, &mut cluster_buffer)?;

            let entries_per_cluster = cluster_size / mem::size_of::<DirectoryEntry>();
            let mut lfn_entries: Vec<LongFilenameEntry> = Vec::new();

            for i in 0..entries_per_cluster {
                let entry_offset = i * mem::size_of::<DirectoryEntry>();
                let entry = unsafe {
                    *(cluster_buffer.as_ptr().add(entry_offset) as *const DirectoryEntry)
                };

                // Check if this is the end of directory entries
                if entry.name[0] == 0x00 {
                    return Ok(entries);
                }

                // Skip deleted entries
                if entry.name[0] == 0xE5 {
                    lfn_entries.clear(); // Clear any partial LFN sequence
                    continue;
                }

                // Check if this is a long filename entry
                if entry.attributes == attributes::LONG_NAME {
                    let lfn_entry = unsafe {
                        *(cluster_buffer.as_ptr().add(entry_offset) as *const LongFilenameEntry)
                    };
                    lfn_entries.push(lfn_entry);
                    continue;
                }

                // This is a regular directory entry
                let long_filename = if !lfn_entries.is_empty() {
                    // Reconstruct long filename from LFN entries
                    let reconstructed = self.reconstruct_long_filename(&lfn_entries, &entry)?;
                    lfn_entries.clear();
                    reconstructed
                } else {
                    None
                };

                entries.push((long_filename, entry));
            }

            // Get the next cluster in the chain
            let next_cluster = self.get_next_cluster(current_cluster)?;
            if next_cluster >= cluster_values::END_OF_CHAIN {
                break;
            }
            current_cluster = next_cluster;
        }

        Ok(entries)
    }

    /// Reconstruct long filename from LFN entries
    fn reconstruct_long_filename(
        &self,
        lfn_entries: &[LongFilenameEntry],
        dir_entry: &DirectoryEntry,
    ) -> Result<Option<String>, &'static str> {
        if lfn_entries.is_empty() {
            return Ok(None);
        }

        // Calculate expected checksum
        let expected_checksum = self.calculate_checksum(&dir_entry.name);

        // Sort LFN entries by sequence number
        let mut sorted_entries = lfn_entries.to_vec();
        sorted_entries.sort_by_key(|entry| entry.sequence & 0x3F);

        // Verify checksum consistency
        for lfn_entry in &sorted_entries {
            if lfn_entry.checksum != expected_checksum {
                crate::serial_println!(
                    "DEBUG: Checksum mismatch! Expected: 0x{:02X}, Got: 0x{:02X}",
                    expected_checksum,
                    lfn_entry.checksum
                );
                return Ok(None); // Checksum mismatch, ignore LFN
            }
        }

        // Reconstruct filename
        let mut filename_utf16 = Vec::new();
        for lfn_entry in sorted_entries {
            // Process in sequence order (1, 2, 3...) for proper filename reconstruction
            // Extract characters from the three name fields (using byte-level access)
            let entry_bytes = unsafe {
                core::slice::from_raw_parts(
                    &lfn_entry as *const LongFilenameEntry as *const u8,
                    mem::size_of::<LongFilenameEntry>(),
                )
            };

            let mut found_terminator = false;

            // name1 starts at offset 1 (after sequence byte), 10 bytes
            for i in 0..5 {
                let offset = 1 + i * 2;
                let ch = u16::from_le_bytes([entry_bytes[offset], entry_bytes[offset + 1]]);
                if ch == 0 {
                    found_terminator = true;
                    break;
                }
                if ch == 0xFFFF {
                    break;
                }
                filename_utf16.push(ch);
            }

            if found_terminator {
                break;
            }

            // name2 starts at offset 14 (after name1 + attr + reserved + checksum), 12 bytes
            for i in 0..6 {
                let offset = 14 + i * 2;
                let ch = u16::from_le_bytes([entry_bytes[offset], entry_bytes[offset + 1]]);
                if ch == 0 {
                    found_terminator = true;
                    break;
                }
                if ch == 0xFFFF {
                    break;
                }
                filename_utf16.push(ch);
            }

            if found_terminator {
                break;
            }

            // name3 starts at offset 28 (after name2 + first_cluster), 4 bytes
            for i in 0..2 {
                let offset = 28 + i * 2;
                let ch = u16::from_le_bytes([entry_bytes[offset], entry_bytes[offset + 1]]);
                if ch == 0 {
                    found_terminator = true;
                    break;
                }
                if ch == 0xFFFF {
                    break;
                }
                filename_utf16.push(ch);
            }

            if found_terminator {
                break;
            }
        }

        let result = self.utf16_to_utf8(&filename_utf16);

        Ok(Some(result))
    }

    /// Convert a directory entry to a FileEntry with long filename support
    fn entry_to_file_entry_with_lfn(
        &self,
        long_filename: Option<String>,
        entry: &DirectoryEntry,
    ) -> FileEntry {
        let name = if let Some(lfn) = long_filename {
            lfn
        } else {
            // Fall back to 8.3 name
            self.name_8_3_to_string(&entry.name)
        };

        let first_cluster =
            ((entry.first_cluster_high as u32) << 16) | (entry.first_cluster_low as u32);

        let created_date = self.raw_date_to_date(entry.creation_date);
        let mut created_time = self.raw_time_to_time(entry.creation_time);
        created_time.add_millis(entry.creation_time_tenths as u32 * 10); // Creation time in hundredths of a second, although the official FAT Specification from Microsoft says it is tenths of a second. Range 0-199 inclusive.

        let last_access_date = self.raw_date_to_date(entry.last_access_date);

        let last_write_date = self.raw_date_to_date(entry.last_write_date);
        let last_write_time = self.raw_time_to_time(entry.last_write_time);

        FileEntry {
            name,
            is_directory: (entry.attributes & attributes::DIRECTORY) != 0,
            size: entry.file_size,
            first_cluster,
            created_at: DateTime::from_date_and_time(created_date, created_time),
            last_access_at: last_access_date,
            last_write_at: DateTime::from_date_and_time(last_write_date, last_write_time),
        }
    }

    /// List files in the root directory
    pub fn list_root_directory(&mut self) -> Result<Vec<FileEntry>, &'static str> {
        let entries_with_lfn =
            self.read_directory_entries_with_lfn(self.boot_sector.root_cluster)?;
        let mut files = Vec::new();

        for (long_filename, entry) in entries_with_lfn {
            // Skip volume labels and system files
            if (entry.attributes & attributes::VOLUME_ID) != 0 {
                continue;
            }

            files.push(self.entry_to_file_entry_with_lfn(long_filename, &entry));
        }

        Ok(files)
    }

    /// List files in a specific directory
    pub fn list_directory(&mut self, dir_cluster: u32) -> Result<Vec<FileEntry>, &'static str> {
        let entries_with_lfn = self.read_directory_entries_with_lfn(dir_cluster)?;
        let mut files = Vec::new();

        for (long_filename, entry) in entries_with_lfn {
            // Skip volume labels
            if (entry.attributes & attributes::VOLUME_ID) != 0 {
                continue;
            }

            files.push(self.entry_to_file_entry_with_lfn(long_filename, &entry));
        }

        Ok(files)
    }

    /// Read a file's content
    pub fn read_file(
        &mut self,
        first_cluster: u32,
        file_size: u32,
    ) -> Result<Vec<u8>, &'static str> {
        let mut file_data = Vec::new();
        let mut current_cluster = first_cluster;
        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let mut bytes_read = 0u32;

        while bytes_read < file_size {
            let mut cluster_buffer = vec![0u8; cluster_size];
            self.read_cluster(current_cluster, &mut cluster_buffer)?;

            let bytes_to_read =
                core::cmp::min(cluster_size as u32, file_size - bytes_read) as usize;

            file_data.extend_from_slice(&cluster_buffer[..bytes_to_read]);
            bytes_read += bytes_to_read as u32;

            if bytes_read >= file_size {
                break;
            }

            // Get the next cluster
            let next_cluster = self.get_next_cluster(current_cluster)?;
            if next_cluster >= cluster_values::END_OF_CHAIN {
                break;
            }
            current_cluster = next_cluster;
        }

        Ok(file_data)
    }

    /// Find a file in a directory by name (supports both short and long names)
    pub fn find_file_in_directory(
        &mut self,
        dir_cluster: u32,
        filename: &str,
    ) -> Result<Option<FileEntry>, &'static str> {
        let entries_with_lfn = self.read_directory_entries_with_lfn(dir_cluster)?;

        for (long_filename, entry) in entries_with_lfn {
            let file_entry = self.entry_to_file_entry_with_lfn(long_filename.clone(), &entry);

            // Check long filename first, then short filename
            if file_entry.name.to_uppercase() == filename.to_uppercase() {
                return Ok(Some(file_entry));
            }

            // Also check against 8.3 name if no long filename
            if long_filename.is_none() {
                let short_name = self.name_8_3_to_string(&entry.name);
                if short_name.to_uppercase() == filename.to_uppercase() {
                    return Ok(Some(file_entry));
                }
            }
        }

        Ok(None)
    }

    /// Find a file in the root directory by name
    pub fn find_file_in_root(&mut self, filename: &str) -> Result<Option<FileEntry>, &'static str> {
        self.find_file_in_directory(self.boot_sector.root_cluster, filename)
    }

    /// Write a cluster to the disk
    fn write_cluster(&mut self, cluster: u32, buffer: &[u8]) -> Result<(), &'static str> {
        let sector = self.cluster_to_sector(cluster);
        let cluster_size = self.sectors_per_cluster * self.bytes_per_sector;

        if buffer.len() < cluster_size as usize {
            return Err("Buffer too small for cluster");
        }

        for i in 0..self.sectors_per_cluster {
            let sector_offset = i * self.bytes_per_sector as u64;
            self.disk.write_sector(
                sector + i,
                &buffer[sector_offset as usize..(sector_offset + self.bytes_per_sector) as usize],
            )?;
        }

        Ok(())
    }

    /// Find a free cluster in the FAT
    fn find_free_cluster(&mut self) -> Result<u32, &'static str> {
        // Start searching from cluster 2 (first data cluster)
        let mut cluster = 2u32;
        let max_clusters = (self.boot_sector.total_sectors_32 - self.data_start_sector as u32)
            / self.boot_sector.sectors_per_cluster as u32;

        while cluster < max_clusters {
            let fat_entry = self.get_next_cluster(cluster)?;
            if fat_entry == cluster_values::FREE {
                return Ok(cluster);
            }
            cluster += 1;
        }

        Err("No free clusters available")
    }

    /// Update a FAT entry
    fn update_fat_entry(&mut self, cluster: u32, value: u32) -> Result<(), &'static str> {
        let fat_offset = cluster * 4; // 4 bytes per FAT32 entry
        let fat_sector = self.fat_start_sector + (fat_offset as u64 / self.bytes_per_sector);
        let sector_offset = (fat_offset as u64 % self.bytes_per_sector) as usize;

        // Read the sector
        let mut sector_buffer = [0u8; 512];
        self.disk.read_sector(fat_sector, &mut sector_buffer)?;

        // Update the FAT entry (preserve upper 4 bits)
        let masked_value = value & cluster_values::MASK;
        let existing_upper = u32::from_le_bytes([
            sector_buffer[sector_offset],
            sector_buffer[sector_offset + 1],
            sector_buffer[sector_offset + 2],
            sector_buffer[sector_offset + 3],
        ]) & !cluster_values::MASK;

        let new_value = existing_upper | masked_value;
        let bytes = new_value.to_le_bytes();

        sector_buffer[sector_offset] = bytes[0];
        sector_buffer[sector_offset + 1] = bytes[1];
        sector_buffer[sector_offset + 2] = bytes[2];
        sector_buffer[sector_offset + 3] = bytes[3];

        // Write back to both FAT copies
        for fat_copy in 0..self.boot_sector.fat_count {
            let fat_sector_copy = self.fat_start_sector
                + (fat_copy as u64 * self.boot_sector.sectors_per_fat_32 as u64)
                + (fat_offset as u64 / self.bytes_per_sector);
            self.disk.write_sector(fat_sector_copy, &sector_buffer)?;
        }

        Ok(())
    }

    /// Allocate a chain of clusters
    fn allocate_cluster_chain(&mut self, num_clusters: u32) -> Result<u32, &'static str> {
        if num_clusters == 0 {
            return Err("Cannot allocate zero clusters");
        }

        let first_cluster = self.find_free_cluster()?;
        let mut current_cluster = first_cluster;

        // Allocate the requested number of clusters
        for i in 0..num_clusters {
            if i == num_clusters - 1 {
                // Last cluster - mark as end of chain
                self.update_fat_entry(current_cluster, cluster_values::END_OF_CHAIN)?;
            } else {
                // Find next free cluster
                let next_cluster = self.find_free_cluster()?;
                self.update_fat_entry(current_cluster, next_cluster)?;
                current_cluster = next_cluster;
            }
        }

        Ok(first_cluster)
    }

    /// Update an existing file with new data, handling cluster allocation/deallocation
    pub fn update_file(
        &mut self,
        dir_cluster: u32,
        filename: &str,
        new_data: &[u8],
    ) -> Result<(), &'static str> {
        // Find the existing file
        let file_entry = match self.find_file_in_directory(dir_cluster, filename)? {
            Some(entry) => entry,
            None => {
                return Err("File not found in directory");
            }
        };

        if file_entry.is_directory {
            return Err("Cannot update directory as file");
        }

        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let old_clusters_needed = if file_entry.size == 0 {
            0
        } else {
            ((file_entry.size as usize + cluster_size - 1) / cluster_size) as u32
        };
        let new_clusters_needed = if new_data.is_empty() {
            0
        } else {
            ((new_data.len() + cluster_size - 1) / cluster_size) as u32
        };

        let new_first_cluster = if new_data.is_empty() {
            // New file is empty, free all existing clusters
            if file_entry.first_cluster >= 2 {
                self.free_cluster_chain(file_entry.first_cluster)?;
            }
            0
        } else if file_entry.first_cluster == 0 {
            // File was empty, allocate new clusters
            let first_cluster = self.allocate_cluster_chain(new_clusters_needed)?;
            self.write_file(first_cluster, new_data)?;
            first_cluster
        } else if new_clusters_needed == old_clusters_needed {
            // Same number of clusters, just overwrite
            self.write_file(file_entry.first_cluster, new_data)?;
            file_entry.first_cluster
        } else if new_clusters_needed < old_clusters_needed {
            // Fewer clusters needed, write data and free excess clusters
            self.write_file(file_entry.first_cluster, new_data)?;

            // Find the last cluster we need to keep
            let mut current_cluster = file_entry.first_cluster;
            for _ in 1..new_clusters_needed {
                current_cluster = self.get_next_cluster(current_cluster)?;
            }

            // Get the next cluster (first one to free)
            let first_cluster_to_free = self.get_next_cluster(current_cluster)?;

            // Mark the last kept cluster as end of chain
            self.update_fat_entry(current_cluster, cluster_values::END_OF_CHAIN)?;

            // Free the remaining clusters
            if first_cluster_to_free < cluster_values::END_OF_CHAIN {
                self.free_cluster_chain(first_cluster_to_free)?;
            }

            file_entry.first_cluster
        } else {
            // More clusters needed, write to existing and allocate more

            // Write data to existing clusters first
            let bytes_in_existing_clusters = old_clusters_needed as usize * cluster_size;
            let bytes_to_write_existing =
                core::cmp::min(new_data.len(), bytes_in_existing_clusters);

            if bytes_to_write_existing > 0 {
                self.write_file(
                    file_entry.first_cluster,
                    &new_data[..bytes_to_write_existing],
                )?;
            }

            // If we need more space, allocate additional clusters
            if new_data.len() > bytes_in_existing_clusters {
                let additional_clusters = new_clusters_needed - old_clusters_needed;
                let new_clusters_start = self.allocate_cluster_chain(additional_clusters)?;

                // Find the last cluster in the existing chain
                let mut last_existing_cluster = file_entry.first_cluster;
                for _ in 1..old_clusters_needed {
                    last_existing_cluster = self.get_next_cluster(last_existing_cluster)?;
                }

                // Link the new clusters to the existing chain
                self.update_fat_entry(last_existing_cluster, new_clusters_start)?;

                // Write remaining data to new clusters
                let remaining_data = &new_data[bytes_in_existing_clusters..];
                self.write_file(new_clusters_start, remaining_data)?;
            }

            file_entry.first_cluster
        };

        // Update the directory entry with new file size and cluster
        self.update_directory_entry(
            dir_cluster,
            filename,
            new_first_cluster,
            new_data.len() as u32,
        )?;

        Ok(())
    }

    /// Write file data to allocated clusters
    pub fn write_file(&mut self, first_cluster: u32, data: &[u8]) -> Result<(), &'static str> {
        // Safety check: don't write to invalid clusters
        if first_cluster < 2 {
            return Err("Invalid cluster number for writing (cannot write to boot sector or FAT)");
        }

        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let mut current_cluster = first_cluster;
        let mut bytes_written = 0;

        while bytes_written < data.len() {
            let mut cluster_buffer = vec![0u8; cluster_size];

            let bytes_to_write = core::cmp::min(cluster_size, data.len() - bytes_written);
            cluster_buffer[..bytes_to_write]
                .copy_from_slice(&data[bytes_written..bytes_written + bytes_to_write]);

            self.write_cluster(current_cluster, &cluster_buffer)?;
            bytes_written += bytes_to_write;

            if bytes_written >= data.len() {
                break;
            }

            // Get next cluster
            let next_cluster = self.get_next_cluster(current_cluster)?;
            if next_cluster >= cluster_values::END_OF_CHAIN {
                return Err("Unexpected end of cluster chain while writing");
            }
            current_cluster = next_cluster;
        }

        Ok(())
    }

    /// Create a new file with the given name and data
    pub fn create_file(
        &mut self,
        dir_cluster: u32,
        filename: &str,
        data: &[u8],
    ) -> Result<(), &'static str> {
        // Check if file already exists
        if self
            .find_file_in_directory(dir_cluster, filename)?
            .is_some()
        {
            return Err("File already exists");
        }

        let first_cluster = if data.is_empty() {
            0
        } else {
            // Calculate number of clusters needed
            let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
            let num_clusters = (data.len() + cluster_size - 1) / cluster_size;

            if num_clusters == 0 {
                return Err("Cannot create empty file");
            }

            // Allocate clusters for the file
            let first_cluster = self.allocate_cluster_chain(num_clusters as u32)?;

            // Write the file data
            self.write_file(first_cluster, data)?;

            first_cluster
        };

        // Create directory entry
        self.create_directory_entry(
            dir_cluster,
            filename,
            first_cluster,
            data.len() as u32,
            false,
        )?;

        Ok(())
    }

    /// Create LFN entries for a long filename
    fn create_lfn_entries(&self, long_name: &str, checksum: u8) -> Vec<LongFilenameEntry> {
        let utf16_name = self.utf8_to_utf16(long_name);
        let mut lfn_entries = Vec::new();

        // Each LFN entry holds 13 characters (5 + 6 + 2)
        let chars_per_entry = 13;
        let num_entries = (utf16_name.len() + chars_per_entry - 1) / chars_per_entry;

        for i in 0..num_entries {
            let start_idx = i * chars_per_entry;
            let end_idx = core::cmp::min(start_idx + chars_per_entry, utf16_name.len());

            let mut name1 = [0xFFFFu16; 5];
            let mut name2 = [0xFFFFu16; 6];
            let mut name3 = [0xFFFFu16; 2];

            let chunk = &utf16_name[start_idx..end_idx];
            let mut char_idx = 0;

            // Fill name1 (5 chars)
            for j in 0..5 {
                if char_idx < chunk.len() {
                    name1[j] = chunk[char_idx];
                    char_idx += 1;
                } else if char_idx == chunk.len() {
                    name1[j] = 0; // Null terminator
                    char_idx += 1;
                } else {
                    name1[j] = 0xFFFF; // Padding
                }
            }

            // Fill name2 (6 chars)
            for j in 0..6 {
                if char_idx < chunk.len() {
                    name2[j] = chunk[char_idx];
                    char_idx += 1;
                } else if char_idx == chunk.len() {
                    name2[j] = 0; // Null terminator
                    char_idx += 1;
                } else {
                    name2[j] = 0xFFFF; // Padding
                }
            }

            // Fill name3 (2 chars)
            for j in 0..2 {
                if char_idx < chunk.len() {
                    name3[j] = chunk[char_idx];
                    char_idx += 1;
                } else if char_idx == chunk.len() {
                    name3[j] = 0; // Null terminator
                    char_idx += 1;
                } else {
                    name3[j] = 0xFFFF; // Padding
                }
            }

            let sequence = if i == num_entries - 1 {
                (i + 1) as u8 | 0x40 // Last entry has bit 6 set
            } else {
                (i + 1) as u8
            };

            let mut lfn_entry = LongFilenameEntry {
                sequence,
                name1: [0; 5],
                attributes: attributes::LONG_NAME,
                reserved: 0,
                checksum,
                name2: [0; 6],
                first_cluster: 0,
                name3: [0; 2],
            };

            // Use byte-level operations to avoid packed field issues
            let entry_bytes = unsafe {
                core::slice::from_raw_parts_mut(
                    &mut lfn_entry as *mut LongFilenameEntry as *mut u8,
                    mem::size_of::<LongFilenameEntry>(),
                )
            };

            // Copy name1 to offset 1, 10 bytes (5 UTF-16 chars)
            for j in 0..5 {
                let offset = 1 + j * 2;
                let bytes = name1[j].to_le_bytes();
                entry_bytes[offset] = bytes[0];
                entry_bytes[offset + 1] = bytes[1];
            }

            // Copy name2 to offset 14, 12 bytes (6 UTF-16 chars)
            for j in 0..6 {
                let offset = 14 + j * 2;
                let bytes = name2[j].to_le_bytes();
                entry_bytes[offset] = bytes[0];
                entry_bytes[offset + 1] = bytes[1];
            }

            // Copy name3 to offset 28, 4 bytes (2 UTF-16 chars)
            for j in 0..2 {
                let offset = 28 + j * 2;
                let bytes = name3[j].to_le_bytes();
                entry_bytes[offset] = bytes[0];
                entry_bytes[offset + 1] = bytes[1];
            }
            lfn_entries.push(lfn_entry);
        }

        // LFN entries need to be in reverse order (highest sequence first)
        lfn_entries.reverse();
        lfn_entries
    }

    /// Add LFN entries and directory entry to a directory
    fn add_lfn_directory_entry(
        &mut self,
        dir_cluster: u32,
        filename: &str,
        dir_entry: &DirectoryEntry,
    ) -> Result<(), &'static str> {
        // Check if we need LFN entries (filename longer than 8.3 or contains special chars)
        let needs_lfn = filename.len() > 12
            || filename.contains(' ')
            || filename.chars().any(|c| !c.is_ascii() || c.is_lowercase())
            || (filename.contains('.') && filename.matches('.').count() > 1);

        if !needs_lfn {
            // Just add the regular directory entry - find a slot and write it
            let slots = self.find_directory_entry_slots(dir_cluster, 1)?;
            let (cluster, slot_index) = slots[0];
            self.write_directory_entry_at_slot(cluster, slot_index, dir_entry)?;
            return Ok(());
        }

        // Create LFN entries
        let checksum = self.calculate_checksum(&dir_entry.name);
        let lfn_entries = self.create_lfn_entries(filename, checksum);

        // Find space for all entries (LFN entries + 1 directory entry)
        let total_entries_needed = lfn_entries.len() + 1;
        let entry_slots = self.find_directory_entry_slots(dir_cluster, total_entries_needed)?;

        // Write LFN entries first
        for (i, lfn_entry) in lfn_entries.iter().enumerate() {
            let slot_cluster = entry_slots[i].0;
            let slot_index = entry_slots[i].1;
            self.write_directory_entry_at_slot(slot_cluster, slot_index, lfn_entry)?;
        }

        // Write the actual directory entry last
        let final_slot_cluster = entry_slots[lfn_entries.len()].0;
        let final_slot_index = entry_slots[lfn_entries.len()].1;
        self.write_directory_entry_at_slot(final_slot_cluster, final_slot_index, dir_entry)?;

        Ok(())
    }

    /// Find available directory entry slots
    fn find_directory_entry_slots(
        &mut self,
        dir_cluster: u32,
        num_slots: usize,
    ) -> Result<Vec<(u32, usize)>, &'static str> {
        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let entries_per_cluster = cluster_size / mem::size_of::<DirectoryEntry>();
        let mut current_cluster = dir_cluster;
        let mut slots = Vec::new();

        loop {
            let cluster_buffer = {
                let mut buffer = vec![0u8; cluster_size];
                self.read_cluster(current_cluster, &mut buffer)?;
                buffer
            };

            for i in 0..entries_per_cluster {
                let entry_offset = i * mem::size_of::<DirectoryEntry>();
                let entry = unsafe {
                    *(cluster_buffer.as_ptr().add(entry_offset) as *const DirectoryEntry)
                };

                // Check if this slot is available
                if entry.name[0] == 0x00 || entry.name[0] == 0xE5 {
                    slots.push((current_cluster, i));
                    if slots.len() >= num_slots {
                        return Ok(slots);
                    }
                }

                // If we hit end of directory, we can use remaining slots
                if entry.name[0] == 0x00 {
                    // Fill remaining slots in this cluster
                    for j in (i + 1)..entries_per_cluster {
                        slots.push((current_cluster, j));
                        if slots.len() >= num_slots {
                            return Ok(slots);
                        }
                    }
                    break;
                }
            }

            // Try next cluster or allocate new one
            let next_cluster = self.get_next_cluster(current_cluster)?;
            if next_cluster >= cluster_values::END_OF_CHAIN {
                if slots.len() < num_slots {
                    // Need to allocate a new cluster
                    let new_cluster = self.find_free_cluster()?;
                    self.update_fat_entry(current_cluster, new_cluster)?;
                    self.update_fat_entry(new_cluster, cluster_values::END_OF_CHAIN)?;

                    // Add slots from the new cluster
                    for i in 0..entries_per_cluster {
                        slots.push((new_cluster, i));
                        if slots.len() >= num_slots {
                            return Ok(slots);
                        }
                    }
                }
                break;
            }
            current_cluster = next_cluster;
        }

        if slots.len() >= num_slots {
            Ok(slots)
        } else {
            Err("Could not find enough directory entry slots")
        }
    }

    /// Write a directory entry at a specific slot
    fn write_directory_entry_at_slot<T>(
        &mut self,
        cluster: u32,
        slot_index: usize,
        entry: &T,
    ) -> Result<(), &'static str> {
        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let mut cluster_buffer = vec![0u8; cluster_size];

        // Read the cluster
        self.read_cluster(cluster, &mut cluster_buffer)?;

        // Write the entry
        let entry_offset = slot_index * mem::size_of::<DirectoryEntry>();
        let entry_size = mem::size_of::<T>();
        let entry_bytes =
            unsafe { core::slice::from_raw_parts(entry as *const T as *const u8, entry_size) };

        cluster_buffer[entry_offset..entry_offset + entry_size].copy_from_slice(entry_bytes);

        // Write the cluster back
        self.write_cluster(cluster, &cluster_buffer)?;

        Ok(())
    }

    /// Create a directory entry with long filename support
    fn create_directory_entry(
        &mut self,
        dir_cluster: u32,
        filename: &str,
        first_cluster: u32,
        file_size: u32,
        is_directory: bool,
    ) -> Result<(), &'static str> {
        // Generate 8.3 name (either from short filename or auto-generated)
        let name_8_3 = if filename.len() <= 12
            && !filename.contains(' ')
            && filename.chars().all(|c| c.is_ascii() && !c.is_lowercase())
            && filename.matches('.').count() <= 1
        {
            // Simple filename that fits 8.3 format
            self.format_filename_8_3(filename)
        } else {
            // Need to generate a short name
            self.generate_short_name(dir_cluster, filename)?
        };

        let now = get_utc_time(); // TODO: Use correct timezone

        let creation_date = self.date_to_raw_date(now.to_date());
        let creation_time = self.time_to_raw_time(now.to_time());
        let creation_time_tenths = (now.to_time().seconds % 2) * 100;
        let last_access_date = creation_date;
        let last_write_date = creation_date;
        let last_write_time = creation_time;

        // Create the directory entry
        let entry = DirectoryEntry {
            name: name_8_3,
            attributes: if is_directory {
                attributes::DIRECTORY
            } else {
                attributes::ARCHIVE
            },
            reserved: 0,
            creation_time_tenths,
            creation_time,
            creation_date,
            last_access_date,
            first_cluster_high: (first_cluster >> 16) as u16,
            last_write_time,
            last_write_date,
            first_cluster_low: (first_cluster & 0xFFFF) as u16,
            file_size,
        };

        // Add the entry with LFN support
        self.add_lfn_directory_entry(dir_cluster, filename, &entry)?;

        Ok(())
    }

    /// Convert filename to 8.3 format (legacy method for simple names)
    fn format_filename_8_3(&self, filename: &str) -> [u8; 11] {
        let mut name_8_3 = [0x20u8; 11]; // Fill with spaces

        let filename_upper = filename.to_uppercase();
        let parts: Vec<&str> = filename_upper.split('.').collect();

        // Handle name part (up to 8 characters)
        let name_part = parts[0];
        let name_len = core::cmp::min(name_part.len(), 8);
        name_8_3[..name_len].copy_from_slice(&name_part.as_bytes()[..name_len]);

        // Handle extension part (up to 3 characters)
        if parts.len() > 1 {
            let ext_part = parts[1];
            let ext_len = core::cmp::min(ext_part.len(), 3);
            name_8_3[8..8 + ext_len].copy_from_slice(&ext_part.as_bytes()[..ext_len]);
        }

        name_8_3
    }

    /// Update a directory entry with new file information
    fn update_directory_entry(
        &mut self,
        dir_cluster: u32,
        filename: &str,
        new_first_cluster: u32,
        new_file_size: u32,
    ) -> Result<(), &'static str> {
        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let entries_per_cluster = cluster_size / mem::size_of::<DirectoryEntry>();
        let mut current_cluster = dir_cluster;

        loop {
            let mut cluster_buffer = vec![0u8; cluster_size];
            self.read_cluster(current_cluster, &mut cluster_buffer)?;

            let mut lfn_entries: Vec<LongFilenameEntry> = Vec::new();

            for i in 0..entries_per_cluster {
                let entry_offset = i * mem::size_of::<DirectoryEntry>();
                let entry = unsafe {
                    *(cluster_buffer.as_ptr().add(entry_offset) as *const DirectoryEntry)
                };

                if entry.name[0] == 0x00 {
                    return Err("File not found in directory");
                }

                // Skip deleted entries
                if entry.name[0] == 0xE5 {
                    lfn_entries.clear(); // Clear any partial LFN sequence
                    continue;
                }

                // Check if this is a long filename entry
                if entry.attributes == attributes::LONG_NAME {
                    let lfn_entry = unsafe {
                        *(cluster_buffer.as_ptr().add(entry_offset) as *const LongFilenameEntry)
                    };
                    lfn_entries.push(lfn_entry);
                    continue;
                }

                // This is a regular directory entry
                let long_filename = if !lfn_entries.is_empty() {
                    // Reconstruct long filename from LFN entries
                    let reconstructed = self.reconstruct_long_filename(&lfn_entries, &entry)?;
                    lfn_entries.clear();
                    reconstructed
                } else {
                    None
                };

                let entry_file = self.entry_to_file_entry_with_lfn(long_filename, &entry);

                if entry_file.name.to_uppercase() == filename.to_uppercase() {
                    let now = get_utc_time(); // TODO: Use correct timezone

                    // Create a mutable copy of the entry
                    let mut updated_entry = entry;
                    updated_entry.first_cluster_high = (new_first_cluster >> 16) as u16;
                    updated_entry.first_cluster_low = (new_first_cluster & 0xFFFF) as u16;
                    updated_entry.file_size = new_file_size;
                    updated_entry.last_write_date = self.date_to_raw_date(now.to_date());
                    updated_entry.last_write_time = self.time_to_raw_time(now.to_time());
                    updated_entry.last_access_date = self.date_to_raw_date(now.to_date());

                    // Write the updated entry back
                    let entry_bytes = unsafe {
                        core::slice::from_raw_parts(
                            &updated_entry as *const DirectoryEntry as *const u8,
                            mem::size_of::<DirectoryEntry>(),
                        )
                    };

                    cluster_buffer[entry_offset..entry_offset + mem::size_of::<DirectoryEntry>()]
                        .copy_from_slice(entry_bytes);

                    self.write_cluster(current_cluster, &cluster_buffer)?;
                    return Ok(());
                }
            }

            let next_cluster = self.get_next_cluster(current_cluster)?;
            if next_cluster >= cluster_values::END_OF_CHAIN {
                break;
            }
            current_cluster = next_cluster;
        }

        Err("File not found in directory")
    }

    /// Delete a file
    pub fn delete_file(&mut self, dir_cluster: u32, filename: &str) -> Result<(), &'static str> {
        // Find the file
        let file_entry = match self.find_file_in_directory(dir_cluster, filename)? {
            Some(entry) => entry,
            None => return Err("File not found"),
        };

        if file_entry.is_directory {
            return Err("Cannot delete directory using delete_file");
        }

        // Free the cluster chain (only if the file has allocated clusters)
        if file_entry.first_cluster > 0 {
            self.free_cluster_chain(file_entry.first_cluster)?;
        }

        // Mark directory entry as deleted
        self.mark_directory_entry_deleted(dir_cluster, filename)?;

        Ok(())
    }

    /// Free a cluster chain
    fn free_cluster_chain(&mut self, first_cluster: u32) -> Result<(), &'static str> {
        if first_cluster < 2 {
            return Err("Invalid cluster number for freeing");
        }

        let mut current_cluster = first_cluster;
        let mut iteration_count = 0;

        while current_cluster >= 2 && current_cluster < cluster_values::END_OF_CHAIN {
            iteration_count += 1;

            // Safety check to prevent infinite loops
            if iteration_count > 10000 {
                serial_println!(
                    "[ERROR] free_cluster_chain: Infinite loop detected! Breaking after {} iterations",
                    iteration_count
                );
                return Err("Infinite loop detected in cluster chain");
            }

            // Get the next cluster BEFORE freeing the current one
            let next_cluster = self.get_next_cluster(current_cluster)?;
            // Now free the current cluster
            self.update_fat_entry(current_cluster, cluster_values::FREE)?;

            // Check if we've reached the end of the chain
            if next_cluster >= cluster_values::END_OF_CHAIN {
                break;
            }

            // Additional safety check for invalid clusters
            if next_cluster < 2 {
                break;
            }

            // Move to the next cluster
            current_cluster = next_cluster;
        }

        Ok(())
    }

    /// Mark a directory entry as deleted (with LFN support)
    fn mark_directory_entry_deleted(
        &mut self,
        dir_cluster: u32,
        filename: &str,
    ) -> Result<(), &'static str> {
        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let entries_per_cluster = cluster_size / mem::size_of::<DirectoryEntry>();
        let mut current_cluster = dir_cluster;

        loop {
            let mut cluster_buffer = vec![0u8; cluster_size];
            self.read_cluster(current_cluster, &mut cluster_buffer)?;

            let mut lfn_start_index: Option<usize> = None;
            let mut lfn_entries: Vec<LongFilenameEntry> = Vec::new();

            for i in 0..entries_per_cluster {
                let entry_offset = i * mem::size_of::<DirectoryEntry>();
                let entry = unsafe {
                    *(cluster_buffer.as_ptr().add(entry_offset) as *const DirectoryEntry)
                };

                if entry.name[0] == 0x00 {
                    return Err("File not found in directory");
                }

                if entry.name[0] == 0xE5 {
                    // Reset LFN tracking for deleted entries
                    lfn_start_index = None;
                    lfn_entries.clear();
                    continue;
                }

                // Check if this is a long filename entry
                if entry.attributes == attributes::LONG_NAME {
                    let lfn_entry = unsafe {
                        *(cluster_buffer.as_ptr().add(entry_offset) as *const LongFilenameEntry)
                    };

                    if lfn_start_index.is_none() {
                        lfn_start_index = Some(i);
                    }
                    lfn_entries.push(lfn_entry);
                    continue;
                }

                // This is a regular directory entry - check if it matches our filename
                let long_filename = if !lfn_entries.is_empty() {
                    self.reconstruct_long_filename(&lfn_entries, &entry)?
                } else {
                    None
                };

                let entry_file = self.entry_to_file_entry_with_lfn(long_filename.clone(), &entry);

                if entry_file.name.to_uppercase() == filename.to_uppercase() {
                    // Found the file! Mark all associated entries as deleted

                    // First, mark any LFN entries as deleted
                    if let Some(start_idx) = lfn_start_index {
                        for lfn_idx in start_idx..i {
                            let lfn_offset = lfn_idx * mem::size_of::<DirectoryEntry>();
                            cluster_buffer[lfn_offset] = 0xE5;
                        }
                    }

                    // Then mark the main directory entry as deleted
                    cluster_buffer[entry_offset] = 0xE5;

                    // Write the updated cluster back to disk
                    self.write_cluster(current_cluster, &cluster_buffer)?;
                    return Ok(());
                }

                // Reset LFN tracking since this entry didn't match
                lfn_start_index = None;
                lfn_entries.clear();
            }

            let next_cluster = self.get_next_cluster(current_cluster)?;
            if next_cluster >= cluster_values::END_OF_CHAIN {
                break;
            }
            current_cluster = next_cluster;
        }

        Err("File not found in directory")
    }

    /// Create a new file in the root directory
    pub fn create_file_in_root(&mut self, filename: &str, data: &[u8]) -> Result<(), &'static str> {
        self.create_file(self.boot_sector.root_cluster, filename, data)
    }

    /// Delete a file from the root directory
    pub fn delete_file_from_root(&mut self, filename: &str) -> Result<(), &'static str> {
        self.delete_file(self.boot_sector.root_cluster, filename)
    }

    /// Update a file in the root directory
    pub fn update_file_in_root(&mut self, filename: &str, data: &[u8]) -> Result<(), &'static str> {
        self.update_file(self.boot_sector.root_cluster, filename, data)
    }

    /// Create a new directory
    pub fn create_directory(
        &mut self,
        parent_cluster: u32,
        dirname: &str,
    ) -> Result<(), &'static str> {
        // Check if directory already exists
        if self
            .find_file_in_directory(parent_cluster, dirname)?
            .is_some()
        {
            return Err("Directory already exists");
        }

        // Allocate a cluster for the new directory
        let dir_cluster = self.allocate_cluster_chain(1)?;

        // Initialize the directory cluster with "." and ".." entries
        self.initialize_directory_cluster(dir_cluster, parent_cluster)?;

        // Create directory entry in parent directory
        self.create_directory_entry(parent_cluster, dirname, dir_cluster, 0, true)?;

        Ok(())
    }

    /// Initialize a directory cluster with "." and ".." entries
    fn initialize_directory_cluster(
        &mut self,
        dir_cluster: u32,
        parent_cluster: u32,
    ) -> Result<(), &'static str> {
        let cluster_size = (self.sectors_per_cluster * self.bytes_per_sector) as usize;
        let mut cluster_buffer = vec![0u8; cluster_size];

        // Create "." entry (current directory)
        let dot_entry = DirectoryEntry {
            name: [
                b'.', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
            ],
            attributes: attributes::DIRECTORY,
            reserved: 0,
            creation_time_tenths: 0,
            creation_time: 0,
            creation_date: 0,
            last_access_date: 0,
            first_cluster_high: (dir_cluster >> 16) as u16,
            last_write_time: 0,
            last_write_date: 0,
            first_cluster_low: (dir_cluster & 0xFFFF) as u16,
            file_size: 0,
        };

        // Create ".." entry (parent directory)
        let dotdot_entry = DirectoryEntry {
            name: [
                b'.', b'.', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
            ],
            attributes: attributes::DIRECTORY,
            reserved: 0,
            creation_time_tenths: 0,
            creation_time: 0,
            creation_date: 0,
            last_access_date: 0,
            first_cluster_high: (parent_cluster >> 16) as u16,
            last_write_time: 0,
            last_write_date: 0,
            first_cluster_low: (parent_cluster & 0xFFFF) as u16,
            file_size: 0,
        };

        // Copy entries to buffer
        let dot_bytes = unsafe {
            core::slice::from_raw_parts(
                &dot_entry as *const DirectoryEntry as *const u8,
                mem::size_of::<DirectoryEntry>(),
            )
        };
        cluster_buffer[..mem::size_of::<DirectoryEntry>()].copy_from_slice(dot_bytes);

        let dotdot_bytes = unsafe {
            core::slice::from_raw_parts(
                &dotdot_entry as *const DirectoryEntry as *const u8,
                mem::size_of::<DirectoryEntry>(),
            )
        };
        cluster_buffer[mem::size_of::<DirectoryEntry>()..2 * mem::size_of::<DirectoryEntry>()]
            .copy_from_slice(dotdot_bytes);

        // Write the initialized cluster
        self.write_cluster(dir_cluster, &cluster_buffer)?;

        Ok(())
    }

    /// Create a new directory in the root directory
    pub fn create_directory_in_root(&mut self, dirname: &str) -> Result<(), &'static str> {
        self.create_directory(self.boot_sector.root_cluster, dirname)
    }

    /// Delete a directory (must be empty)
    pub fn delete_directory(
        &mut self,
        parent_cluster: u32,
        dirname: &str,
    ) -> Result<(), &'static str> {
        // Find the directory
        let dir_entry = match self.find_file_in_directory(parent_cluster, dirname)? {
            Some(entry) => entry,
            None => return Err("Directory not found"),
        };

        if !dir_entry.is_directory {
            return Err("Not a directory");
        }

        // Check if directory is empty (only "." and ".." entries should exist)
        if !self.is_directory_empty(dir_entry.first_cluster)? {
            return Err("Directory not empty");
        }

        // Free the cluster(s) used by the directory
        self.free_cluster_chain(dir_entry.first_cluster)?;

        // Mark directory entry as deleted in parent
        self.mark_directory_entry_deleted(parent_cluster, dirname)?;

        Ok(())
    }

    /// Check if a directory is empty (contains only "." and ".." entries)
    fn is_directory_empty(&mut self, dir_cluster: u32) -> Result<bool, &'static str> {
        let entries = self.read_directory_entries(dir_cluster)?;

        for entry in entries {
            // Skip volume labels and long name entries
            if (entry.attributes & attributes::VOLUME_ID) != 0
                || entry.attributes == attributes::LONG_NAME
            {
                continue;
            }

            // Check if this is "." or ".." entry
            let is_dot = entry.name[0] == b'.' && entry.name[1] == b' ';
            let is_dotdot = entry.name[0] == b'.' && entry.name[1] == b'.' && entry.name[2] == b' ';

            if !is_dot && !is_dotdot {
                return Ok(false); // Found a non-dot entry, directory is not empty
            }
        }

        Ok(true)
    }

    /// Delete a directory from the root directory
    pub fn delete_directory_from_root(&mut self, dirname: &str) -> Result<(), &'static str> {
        self.delete_directory(self.boot_sector.root_cluster, dirname)
    }

    /// Navigate to a subdirectory and return its cluster number
    pub fn navigate_to_directory(
        &mut self,
        current_cluster: u32,
        dirname: &str,
    ) -> Result<u32, &'static str> {
        // Handle special cases
        if dirname == "." {
            return Ok(current_cluster);
        }

        if dirname == ".." {
            // Find the parent directory by reading the ".." entry
            let entries = self.read_directory_entries(current_cluster)?;
            for entry in entries {
                let is_dotdot =
                    entry.name[0] == b'.' && entry.name[1] == b'.' && entry.name[2] == b' ';
                if is_dotdot {
                    let parent_cluster = ((entry.first_cluster_high as u32) << 16)
                        | (entry.first_cluster_low as u32);
                    return Ok(parent_cluster);
                }
            }
            return Err("Parent directory not found");
        }

        // Find the directory entry
        let dir_entry = match self.find_file_in_directory(current_cluster, dirname)? {
            Some(entry) => entry,
            None => return Err("Directory not found"),
        };

        if !dir_entry.is_directory {
            return Err("Not a directory");
        }

        Ok(dir_entry.first_cluster)
    }

    /// Check if a file exists in a directory
    pub fn is_file(&mut self, current_cluster: u32, filename: &str) -> Result<bool, &'static str> {
        if let Some(entry) = self.find_file_in_directory(current_cluster, filename)? {
            Ok(!entry.is_directory)
        } else {
            Ok(false)
        }
    }

    /// Check if a directory exists in a directory
    pub fn is_directory(
        &mut self,
        current_cluster: u32,
        dirname: &str,
    ) -> Result<bool, &'static str> {
        if let Some(entry) = self.find_file_in_directory(current_cluster, dirname)? {
            Ok(entry.is_directory)
        } else {
            Ok(false)
        }
    }

    /// Get the root cluster number
    pub fn get_root_cluster(&self) -> u32 {
        self.boot_sector.root_cluster
    }

    /// Check if a cluster is the root directory
    pub fn is_root_directory(&self, cluster: u32) -> bool {
        cluster == self.boot_sector.root_cluster
    }
}
