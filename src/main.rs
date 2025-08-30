use config::{BootMode, CONFIG};

fn main() {
    // read env variables that were set in build script
    let (path, uefi) = if CONFIG.boot_mode == BootMode::Uefi {
        (env!("UEFI_PATH"), true)
    } else {
        (env!("BIOS_PATH"), false)
    };

    let mut cmd = std::process::Command::new("qemu-system-x86_64");
    cmd.arg("-serial").arg("stdio");
    if uefi {
        cmd.arg("-bios").arg(ovmf_prebuilt::ovmf_pure_efi());
        cmd.arg("-drive").arg(format!("format=raw,file={path}"));
    } else {
        cmd.arg("-drive").arg(format!("format=raw,file={path}"));
    }

    cmd.arg("-drive")
        .arg("file=disk.img,format=raw,if=ide,cache=writeback,snapshot=off");

    // Helps us when we reboot bc of a triple fault
    // cmd.arg("-d").arg("int");
    // cmd.arg("-no-reboot");

    let mut child = cmd.spawn().unwrap();
    child.wait().unwrap();
}
