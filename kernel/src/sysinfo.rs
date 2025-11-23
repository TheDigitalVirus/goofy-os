use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use config::CONFIG;

use crate::{
    allocator::ALLOCATOR,
    fs::manager::FILESYSTEM,
    gdt::STACK_SIZE,
    init::{HEAP_SIZE, HEAP_START},
};

pub static mut STACK_BASE: usize = 0;

#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub os_name: String,
    pub os_version: String,
    pub architecture: String,
    pub processor_vendor: String,
    pub processor_model: String,
    pub base_frequency: Option<u16>,
    pub max_frequency: Option<u16>,
    pub heap_size: u64,
    pub heap_start: u64,
    pub heap_used: u64,
    pub stack_size: usize,
    pub cpu_features: Vec<String>,
    pub filesystem_info: Option<FilesystemInfo>,
}

impl SystemInfo {
    pub fn gather() -> Self {
        let heap_info = get_heap_info();
        let cpu_info = get_cpu_info();
        let filesystem_info = get_filesystem_info();

        SystemInfo {
            os_name: CONFIG.os_name.to_string(),
            os_version: CONFIG.os_version.to_string(),
            architecture: CONFIG.architecture.to_string(),
            processor_vendor: cpu_info.vendor_id,
            processor_model: cpu_info.model,
            base_frequency: cpu_info.base_frequency,
            max_frequency: cpu_info.max_frequency,

            heap_size: HEAP_SIZE,
            heap_start: HEAP_START,
            heap_used: heap_info.used_bytes,

            stack_size: STACK_SIZE,
            cpu_features: cpu_info.features,
            filesystem_info,
        }
    }
}

pub struct HeapInfo {
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

pub struct CpuInfo {
    pub features: Vec<String>,
    pub vendor_id: String,
    pub model: String,
    pub base_frequency: Option<u16>,
    pub max_frequency: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct FilesystemInfo {
    pub filesystem_type: String,
    pub total_size: u64,
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub total_clusters: u32,
    pub volume_label: String,
    pub root_entries: u32,
    pub fat_count: u8,
    pub filesystem_version: u16,
}

fn get_heap_info() -> HeapInfo {
    let estimated_used = estimate_heap_usage();

    HeapInfo {
        used_bytes: estimated_used,
        free_bytes: HEAP_SIZE.saturating_sub(estimated_used),
        total_bytes: HEAP_SIZE,
    }
}

pub fn estimate_heap_usage() -> u64 {
    unsafe {
        let allocator = &raw mut ALLOCATOR;
        (*allocator).allocated() as u64
    }
}

fn get_cpu_info() -> CpuInfo {
    use raw_cpuid::CpuId;

    let cpuid = CpuId::new();

    let vendor_id = if let Some(info) = cpuid.get_vendor_info() {
        info.as_str().to_string()
    } else {
        "Unknown".to_string()
    };

    let model = if let Some(info) = cpuid.get_processor_brand_string() {
        info.as_str().to_string()
    } else {
        "Unknown".to_string()
    };

    let (base_frequency, max_frequency) = if let Some(info) = cpuid.get_processor_frequency_info() {
        (
            Some(info.processor_base_frequency()),
            Some(info.processor_max_frequency()),
        )
    } else {
        (None, None)
    };

    let mut features = Vec::new();

    if let Some(finfo) = cpuid.get_feature_info() {
        if finfo.has_sse() {
            features.push("SSE".to_string());
        }
        if finfo.has_sse2() {
            features.push("SSE2".to_string());
        }
        if finfo.has_sse3() {
            features.push("SSE3".to_string());
        }
        if finfo.has_fma() {
            features.push("FMA".to_string());
        }
        if finfo.has_mmx() {
            features.push("MMX".to_string());
        }
        if finfo.has_fpu() {
            features.push("FPU".to_string());
        }
    }

    if let Some(extended) = cpuid.get_extended_feature_info() {
        if extended.has_avx2() {
            features.push("AVX2".to_string());
        }
        if extended.has_bmi1() {
            features.push("BMI1".to_string());
        }
        if extended.has_bmi2() {
            features.push("BMI2".to_string());
        }
    }

    CpuInfo {
        features,
        vendor_id,
        model,
        base_frequency,
        max_frequency,
    }
}

pub fn format_memory_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} bytes", bytes)
    }
}

// Function to get stack pointer (for stack usage estimation)
#[inline(always)]
pub fn get_stack_pointer() -> u64 {
    let sp: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

pub fn estimate_stack_usage() -> usize {
    let current_sp = get_stack_pointer();

    let estimated_used = (unsafe { STACK_BASE } - current_sp as usize);

    // Clamp to reasonable values
    estimated_used.min(4096 * 5) // Max our known stack size
}

fn get_filesystem_info() -> Option<FilesystemInfo> {
    let fs_guard = FILESYSTEM.lock();
    if let Some(fs) = fs_guard.as_ref() {
        Some(fs.get_filesystem_info())
    } else {
        None
    }
}
