use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde::de::Deserializer;
use serde::Deserialize;

use crate::types::{Disk, InstallConfig};

const REQUIRED_COMMANDS: &[&str] = &[
    "id",
    "findmnt",
    "readlink",
    "lsblk",
    "wipefs",
    "sgdisk",
    "partprobe",
    "mkfs.fat",
    "mkswap",
    "swapon",
    "swapoff",
    "mount",
    "umount",
    "zpool",
    "zfs",
    "blkid",
    "nixos-generate-config",
    "udevadm",
];

pub fn preflight_checks() -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("This installer must run on Linux (NixOS installer environment).");
    }

    let euid_out = run_command_capture("id", &["-u"])?;
    if euid_out.trim() != "0" {
        bail!("Please run as root (e.g. sudo nix run ...)");
    }

    if !Path::new("/sys/firmware/efi").exists() {
        bail!(
            "UEFI mode was not detected. Reboot the installer media in UEFI mode before proceeding."
        );
    }

    Ok(())
}

pub fn check_required_commands() -> Result<()> {
    let mut missing = Vec::new();
    for cmd in REQUIRED_COMMANDS {
        if which::which(cmd).is_err() {
            missing.push(*cmd);
        }
    }
    if !missing.is_empty() {
        bail!("Missing required tools in PATH: {}", missing.join(", "));
    }
    Ok(())
}

#[derive(Deserialize)]
struct LsblkResponse {
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Deserialize, Clone)]
struct LsblkDevice {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(rename = "type", default)]
    dev_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_boolish")]
    rm: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_opt_boolish")]
    ro: Option<bool>,
    #[serde(default)]
    partlabel: Option<String>,
    #[serde(default)]
    mountpoints: Option<Vec<Option<String>>>,
    #[serde(default)]
    children: Option<Vec<LsblkDevice>>,
}

pub fn load_disks() -> Result<Vec<Disk>> {
    let out = run_command_capture(
        "lsblk",
        &["-J", "-d", "-o", "NAME,PATH,SIZE,MODEL,TYPE,RM,RO"],
    )?;
    let parsed: LsblkResponse = serde_json::from_str(&out).context("Failed to parse lsblk output")?;

    let disks = parsed
        .blockdevices
        .into_iter()
        .filter(|d| d.dev_type.as_deref().unwrap_or_default() == "disk")
        .filter(|d| !d.ro.unwrap_or(false))
        .filter(|d| !d.rm.unwrap_or(false))
        .map(|d| Disk {
            path: d.path.unwrap_or_default(),
            size: if d.size.as_deref().unwrap_or_default().is_empty() {
                "?".to_string()
            } else {
                d.size.unwrap_or_default()
            },
            model: if d.model.as_deref().unwrap_or_default().trim().is_empty() {
                "(unknown model)".to_string()
            } else {
                d.model.unwrap_or_default().trim().to_string()
            },
        })
        .filter(|d| !d.path.is_empty())
        .collect();

    Ok(disks)
}

fn deserialize_opt_boolish<'de, D>(deserializer: D) -> std::result::Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let parsed = match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Bool(b)) => Some(b),
        Some(serde_json::Value::Number(n)) => Some(n.as_u64().unwrap_or(0) != 0),
        Some(serde_json::Value::String(s)) => {
            let lowered = s.trim().to_ascii_lowercase();
            match lowered.as_str() {
                "1" | "true" | "yes" => Some(true),
                "0" | "false" | "no" | "" => Some(false),
                _ => Some(false),
            }
        }
        _ => Some(false),
    };
    Ok(parsed)
}

pub fn validate_size_input(value: &str, label: &str) -> Result<()> {
    let size_re = Regex::new(r"^[1-9][0-9]*(K|M|G|T)$").unwrap();
    if !size_re.is_match(value) {
        bail!("{} size must match pattern like 512M, 1G, 16G.", label);
    }
    Ok(())
}

pub fn validate_hostname(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        bail!("Hostname cannot be empty.");
    }
    let re = Regex::new(r"^[A-Za-z0-9][A-Za-z0-9-]{0,62}$").unwrap();
    if !re.is_match(value) {
        bail!("Hostname must use letters, digits, or '-', and start with a letter/digit.");
    }
    Ok(())
}

pub fn validate_timezone(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        bail!("Timezone cannot be empty.");
    }
    if value.contains(' ') {
        bail!("Timezone must not contain spaces (example: Europe/Berlin).");
    }
    Ok(())
}

pub fn validate_keyboard_layout(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        bail!("Keyboard layout cannot be empty.");
    }
    let re = Regex::new(r"^[A-Za-z0-9_-]+$").unwrap();
    if !re.is_match(value) {
        bail!("Keyboard layout must use letters, digits, '_' or '-'.");
    }
    Ok(())
}

pub fn validate_username(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        bail!("Username cannot be empty.");
    }
    let re = Regex::new(r"^[a-z_][a-z0-9_-]*$").unwrap();
    if !re.is_match(value) {
        bail!("Username must start with a-z/_ and contain only a-z, 0-9, '_' or '-'.");
    }
    Ok(())
}

pub fn validate_password(value: &str, label: &str) -> Result<()> {
    if value.len() < 8 {
        bail!("{} must be at least 8 characters.", label);
    }
    Ok(())
}

pub fn validate_zfs_ashift(value: &str) -> Result<()> {
    let parsed: u8 = value
        .trim()
        .parse()
        .map_err(|_| anyhow!("ashift must be a number between 9 and 16."))?;
    if !(9..=16).contains(&parsed) {
        bail!("ashift must be between 9 and 16.");
    }
    Ok(())
}

pub fn validate_zfs_redundancy(value: &str) -> Result<()> {
    let v = value.trim().to_ascii_lowercase();
    match v.as_str() {
        "single" | "mirror" | "raidz1" | "raidz2" | "raidz3" => Ok(()),
        _ => bail!("Redundancy must be one of: single, mirror, raidz1, raidz2, raidz3."),
    }
}

pub fn validate_zfs_compression(value: &str) -> Result<()> {
    let v = value.trim().to_ascii_lowercase();
    match v.as_str() {
        "lz4" | "zstd" | "gzip" | "zle" | "on" | "off" => Ok(()),
        _ => bail!("Compression must be one of: lz4, zstd, gzip, zle, on, off."),
    }
}

pub fn validate_zfs_primarycache(value: &str) -> Result<()> {
    let v = value.trim().to_ascii_lowercase();
    match v.as_str() {
        "all" | "metadata" | "none" => Ok(()),
        _ => bail!("Primary cache must be one of: all, metadata, none."),
    }
}

pub fn validate_zfs_options(cfg: &InstallConfig) -> Result<()> {
    validate_zfs_ashift(&cfg.zfs_ashift)?;
    validate_zfs_redundancy(&cfg.zfs_redundancy)?;
    validate_zfs_compression(&cfg.zfs_compression)?;
    validate_zfs_primarycache(&cfg.zfs_primarycache)?;

    let redundancy = cfg.zfs_redundancy.trim().to_ascii_lowercase();
    if redundancy != "single" {
        bail!(
            "Selected redundancy '{}' requires multi-disk pool setup, which is not yet supported in this installer mode. Use 'single'.",
            cfg.zfs_redundancy
        );
    }
    Ok(())
}

pub fn nix_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn has_existing_configuration(disk: &str) -> Result<bool> {
    let labels = run_command_capture("lsblk", &["-J", "-o", "PATH,PARTLABEL", disk])?;
    let parsed: LsblkResponse = serde_json::from_str(&labels)?;
    let mut labels_set = HashSet::new();
    flatten_labels(&parsed.blockdevices, &mut labels_set);

    if labels_set.contains("NIXROOT") || labels_set.contains("NIXBOOT") || labels_set.contains("NIXSWAP") {
        return Ok(true);
    }

    let status = Command::new("zpool")
        .args(["status", "-P", "zroot"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    if let Ok(output) = status {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if zpool_uses_disk(&text, disk) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn flatten_labels(devs: &[LsblkDevice], labels: &mut HashSet<String>) {
    for dev in devs {
        if let Some(label) = &dev.partlabel {
            labels.insert(label.clone());
        }
        if let Some(children) = &dev.children {
            flatten_labels(children, labels);
        }
    }
}

pub fn zpool_uses_disk(zpool_status: &str, disk: &str) -> bool {
    let disk_nvme = format!("{disk}p");
    for line in zpool_status.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(disk) || trimmed.starts_with(&disk_nvme) {
            return true;
        }
    }
    false
}

pub fn collect_disk_warnings(disk: &str) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let out = run_command_capture("lsblk", &["-J", "-o", "PATH,TYPE,MOUNTPOINTS", disk])?;
    let parsed: LsblkResponse = serde_json::from_str(&out)?;

    let mut has_mounts = false;
    flatten_mount_flags(&parsed.blockdevices, &mut has_mounts);
    if has_mounts {
        warnings.push("One or more partitions on this disk are mounted right now.".to_string());
    }

    if is_probable_system_disk(disk)? {
        warnings.push("Selected disk appears to back the currently mounted root filesystem.".to_string());
    }

    Ok(warnings)
}

fn flatten_mount_flags(devs: &[LsblkDevice], has_mounts: &mut bool) {
    for d in devs {
        if let Some(mps) = &d.mountpoints {
            for m in mps {
                if m.as_ref().is_some_and(|v| !v.is_empty()) {
                    *has_mounts = true;
                    return;
                }
            }
        }
        if let Some(children) = &d.children {
            flatten_mount_flags(children, has_mounts);
            if *has_mounts {
                return;
            }
        }
    }
}

fn is_probable_system_disk(disk: &str) -> Result<bool> {
    let root_src = run_command_capture("findmnt", &["-n", "-o", "SOURCE", "/"])?;
    let root_src = root_src.trim();
    if root_src.is_empty() {
        return Ok(false);
    }

    let status = Command::new("lsblk")
        .args(["-no", "PKNAME", root_src])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    if let Ok(output) = status {
        if output.status.success() {
            let pk = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !pk.is_empty() {
                let root_disk = format!("/dev/{pk}");
                return Ok(root_disk == disk);
            }
        }
    }

    Ok(false)
}

pub fn run_command(binary: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(binary)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to execute {binary}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "Command failed: {} {}\nstdout:\n{}\nstderr:\n{}",
            binary,
            args.join(" "),
            stdout.trim(),
            stderr.trim()
        );
    }
    Ok(())
}

pub fn run_command_capture(binary: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(binary)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("Failed to execute {binary}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Command failed: {} {}\n{}", binary, args.join(" "), stderr.trim());
    }

    String::from_utf8(output.stdout).map_err(|e| anyhow!(e))
}

pub fn run_command_allow_fail(binary: &str, args: &[&str]) -> Result<()> {
    let _ = Command::new(binary)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(())
}
