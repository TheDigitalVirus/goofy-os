use config;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(uefi)");
    println!("cargo::rustc-check-cfg=cfg(processes_enabled)");

    if config::CONFIG.boot_mode == config::BootMode::Uefi {
        println!("cargo::rustc-cfg=uefi");
    }

    if config::CONFIG.processes_enabled {
        println!("cargo::rustc-cfg=processes_enabled");
    }
}
