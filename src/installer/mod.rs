use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use rand::Rng;
use tempfile::NamedTempFile;

use crate::system::{
    nix_escape, run_command, run_command_allow_fail, run_command_capture, validate_zfs_options,
    zpool_uses_disk,
};
use crate::types::InstallConfig;

pub fn run_installer(cfg: &InstallConfig) -> Result<()> {
    validate_zfs_options(cfg)?;

    let zfs_ashift = cfg.zfs_ashift.trim().to_string();
    let zfs_compression = cfg.zfs_compression.trim().to_ascii_lowercase();
    let zfs_primarycache = cfg.zfs_primarycache.trim().to_ascii_lowercase();

    println!("-> Cleaning up prior mounts/swap/pools");
    cleanup(cfg)?;

    println!("-> Wiping and partitioning {}", cfg.disk.path);
    run_command("wipefs", &["-a", &cfg.disk.path])?;
    run_command("sgdisk", &["--zap-all", &cfg.disk.path])?;
    run_command(
        "sgdisk",
        &[
            "-n",
            &format!("1:0:+{}", cfg.boot_size),
            "-t",
            "1:ef00",
            "-c",
            "1:NIXBOOT",
            &cfg.disk.path,
        ],
    )?;
    run_command(
        "sgdisk",
        &[
            "-n",
            &format!("2:0:+{}", cfg.swap_size),
            "-t",
            "2:8200",
            "-c",
            "2:NIXSWAP",
            &cfg.disk.path,
        ],
    )?;
    run_command(
        "sgdisk",
        &["-n", "3:0:0", "-t", "3:8300", "-c", "3:NIXROOT", &cfg.disk.path],
    )?;

    run_command("partprobe", &[&cfg.disk.path])?;
    run_command("udevadm", &["settle", "--timeout=30"])?;

    let part_boot = resolve_partition_path("NIXBOOT", &cfg.disk.path)?;
    let part_swap = resolve_partition_path("NIXSWAP", &cfg.disk.path)?;
    let part_zfs = resolve_partition_path("NIXROOT", &cfg.disk.path)?;

    println!("-> Formatting EFI and swap");
    run_command("mkfs.fat", &["-F", "32", "-n", "NIXBOOT", &part_boot])?;
    run_command("mkswap", &["-L", "SWAP", &part_swap])?;
    run_command("swapon", &[&part_swap])?;

    let autotrim_value = if cfg.zfs_autotrim { "on" } else { "off" };
    let mut zpool_args: Vec<String> = vec![
        "create".to_string(),
        "-f".to_string(),
        "-o".to_string(),
        format!("ashift={zfs_ashift}"),
        "-o".to_string(),
        format!("autotrim={autotrim_value}"),
        "-O".to_string(),
        format!("compression={zfs_compression}"),
        "-O".to_string(),
        "acltype=posixacl".to_string(),
        "-O".to_string(),
        "atime=off".to_string(),
        "-O".to_string(),
        "xattr=sa".to_string(),
        "-O".to_string(),
        "normalization=formD".to_string(),
        "-O".to_string(),
        "mountpoint=none".to_string(),
        "-O".to_string(),
        format!("primarycache={zfs_primarycache}"),
    ];

    let mut keyfile_guard: Option<NamedTempFile> = None;

    if cfg.enable_encryption {
        println!("-> Creating encrypted zpool and datasets");
        let mut keyfile = NamedTempFile::new_in("/tmp")?;
        use std::io::Write;
        keyfile.write_all(cfg.passphrase.as_bytes())?;
        keyfile.write_all(b"\n")?;
        let keypath = keyfile.path().to_string_lossy().to_string();

        zpool_args.extend([
            "-O".to_string(),
            "encryption=aes-256-gcm".to_string(),
            "-O".to_string(),
            "keyformat=passphrase".to_string(),
            "-O".to_string(),
            format!("keylocation=file://{keypath}"),
        ]);

        // Hold tempfile until zpool create has consumed key material.
        keyfile_guard = Some(keyfile);
    } else {
        println!("-> Creating unencrypted zpool and datasets");
    }

    zpool_args.push("zroot".to_string());
    zpool_args.push(part_zfs.clone());
    let zpool_arg_refs: Vec<&str> = zpool_args.iter().map(String::as_str).collect();
    run_command("zpool", &zpool_arg_refs)?;
    drop(keyfile_guard);
    if cfg.enable_encryption {
        run_command("zfs", &["set", "keylocation=prompt", "zroot"])?;
    }

    run_command("zfs", &["create", "-o", "mountpoint=legacy", "zroot/root"])?;
    run_command("zfs", &["create", "-o", "mountpoint=legacy", "zroot/nix"])?;
    run_command("zfs", &["create", "-o", "mountpoint=legacy", "zroot/home"])?;
    run_command("zfs", &["create", "-o", "mountpoint=legacy", "zroot/tmp"])?;

    println!("-> Mounting filesystems");
    run_command("mount", &["-t", "zfs", "zroot/root", "/mnt"])?;
    fs::create_dir_all("/mnt/boot")?;
    fs::create_dir_all("/mnt/nix")?;
    fs::create_dir_all("/mnt/home")?;
    fs::create_dir_all("/mnt/tmp")?;
    run_command("mount", &[&part_boot, "/mnt/boot"])?;
    run_command("mount", &["-t", "zfs", "zroot/nix", "/mnt/nix"])?;
    run_command("mount", &["-t", "zfs", "zroot/home", "/mnt/home"])?;
    run_command("mount", &["-t", "zfs", "zroot/tmp", "/mnt/tmp"])?;

    println!("-> Generating NixOS configuration");
    run_command("nixos-generate-config", &["--root", "/mnt"])?;

    println!("-> Injecting zfs.nix overrides");
    let host_id = random_host_id();
    let swap_partuuid = run_command_capture("blkid", &["-s", "PARTUUID", "-o", "value", &part_swap])?;
    let swap_partuuid = swap_partuuid.trim();
    if swap_partuuid.is_empty() {
        bail!("Could not determine PARTUUID for swap partition.");
    }

    let zfs_module = format!(
        "{{ config, pkgs, ... }}:\n\n{{\n  boot.loader.systemd-boot.enable = true;\n  boot.loader.efi.canTouchEfiVariables = true;\n  boot.loader.grub.enable = pkgs.lib.mkForce false;\n\n  networking.hostId = \"{host_id}\";\n  boot.supportedFilesystems = [ \"zfs\" ];\n  boot.zfs.devNodes = \"/dev/disk/by-partlabel\";\n\n  swapDevices = pkgs.lib.mkForce [ {{\n    device = \"/dev/disk/by-partuuid/{swap_partuuid}\";\n    randomEncryption.enable = true;\n  }} ];\n}}\n"
    );
    fs::write("/mnt/etc/nixos/zfs.nix", zfs_module)?;

    let escaped_username = nix_escape(&cfg.username);
    let escaped_user_password = nix_escape(&cfg.user_password);
    let escaped_root_password = nix_escape(&cfg.root_password);
    let escaped_hostname = nix_escape(&cfg.hostname);
    let escaped_timezone = nix_escape(&cfg.timezone);
    let escaped_keyboard_layout = nix_escape(&cfg.keyboard_layout);

    let flakes_snippet = if cfg.enable_flakes {
        "  nix.settings.experimental-features = [ \"nix-command\" \"flakes\" ];\n"
    } else {
        ""
    };
    let user_groups = if cfg.enable_sudo { "[ \"wheel\" ]" } else { "[ ]" };
    let networkmanager_value = if cfg.install_networkmanager { "true" } else { "false" };
    let git_value = if cfg.enable_git { "true" } else { "false" };

    let setup_module = format!(
        "{{ config, pkgs, ... }}:\n\n{{\n  networking.hostName = \"{escaped_hostname}\";\n  time.timeZone = \"{escaped_timezone}\";\n  services.xserver.xkb.layout = \"{escaped_keyboard_layout}\";\n  console.keyMap = \"{escaped_keyboard_layout}\";\n{flakes_snippet}  networking.networkmanager.enable = {networkmanager_value};\n  programs.git.enable = {git_value};\n\n  users.users.\"{escaped_username}\" = {{\n    isNormalUser = true;\n    extraGroups = {user_groups};\n    initialPassword = \"{escaped_user_password}\";\n  }};\n\n  users.users.root.initialPassword = \"{escaped_root_password}\";\n}}\n"
    );
    fs::write("/mnt/etc/nixos/system-setup.nix", setup_module)?;

    let config_path = PathBuf::from("/mnt/etc/nixos/configuration.nix");
    let cfg_text = fs::read_to_string(&config_path)
        .context("Unable to read generated /mnt/etc/nixos/configuration.nix")?;

    let marker = "./hardware-configuration.nix";
    if !cfg_text.contains(marker) {
        bail!(
            "Could not find {marker} import in configuration.nix; please add ./zfs.nix and ./system-setup.nix manually."
        );
    }

    let replaced = cfg_text.replacen(
        marker,
        "./hardware-configuration.nix ./zfs.nix ./system-setup.nix",
        1,
    );
    fs::write(&config_path, replaced)?;

    println!();
    println!("=== ZFS Configuration Complete ===");
    println!("Configuration ready at /mnt/etc/nixos/");
    println!("Review: nano /mnt/etc/nixos/configuration.nix");
    println!("Install: nixos-install");

    Ok(())
}

fn cleanup(cfg: &InstallConfig) -> Result<()> {
    let _ = run_command_allow_fail("umount", &["-R", "/mnt"]);
    let _ = run_command_allow_fail("swapoff", &["-a"]);

    let status = Command::new("zpool")
        .args(["status", "-P", "zroot"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    if let Ok(output) = status {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if zpool_uses_disk(&text, &cfg.disk.path) {
                let _ = run_command_allow_fail("zpool", &["destroy", "-f", "zroot"]);
            }
        }
    }

    run_command("udevadm", &["settle", "--timeout=30"])?;
    Ok(())
}

fn resolve_partition_path(label: &str, disk: &str) -> Result<String> {
    let path = format!("/dev/disk/by-partlabel/{label}");
    for _ in 0..40 {
        if Path::new(&path).exists() {
            let canonical = run_command_capture("readlink", &["-f", &path])?;
            let canonical = canonical.trim().to_string();
            if canonical.starts_with(disk) {
                return Ok(path);
            }
        }
        run_command_allow_fail("udevadm", &["settle", "--timeout=1"])?;
    }
    bail!(
        "Timed out waiting for partition label {label} to appear on selected disk {disk}."
    )
}

fn random_host_id() -> String {
    let mut bytes = [0u8; 4];
    rand::rng().fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
