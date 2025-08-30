use config;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(uefi)");

    if config::CONFIG.boot_mode == config::BootMode::Uefi {
        println!("cargo::rustc-cfg=uefi");
    }
}
