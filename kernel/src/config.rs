pub const IS_UEFI: bool = true;

pub const APIC_ENABLED: bool = IS_UEFI;
pub const LEGACY_PIC_ENABLED: bool = !IS_UEFI;

pub const OS_NAME: &str = "Goofy OS";
pub const OS_VERSION: &str = "0.1.0";
pub const ARCHITECTURE: &str = "x86_64";
