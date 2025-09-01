#![no_std]

use core::cmp::PartialEq;
use core::prelude::rust_2024::derive;

const OS_NAME: &str = "Goofy OS";
const OS_VERSION: &str = "0.1.0";
const ARCHITECTURE: &str = "x86_64";
const PROCESSES_ENABLED: bool = false;

#[derive(PartialEq)]
pub enum BootMode {
    Bios,
    Uefi,
}

pub struct Config {
    pub os_name: &'static str,
    pub os_version: &'static str,
    pub architecture: &'static str,
    pub processes_enabled: bool,
    pub boot_mode: BootMode,
    pub fs_type: FileSystem,
}

#[derive(PartialEq)]
pub enum FileSystem {
    Fat32,
}

pub const CONFIG: Config = Config {
    os_name: OS_NAME,
    os_version: OS_VERSION,
    architecture: ARCHITECTURE,
    processes_enabled: PROCESSES_ENABLED,
    boot_mode: BootMode::Uefi, // or Uefi
    fs_type: FileSystem::Fat32,
};
