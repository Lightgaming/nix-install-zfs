mod installer;
mod system;
mod tui;
mod types;

use anyhow::{bail, Result};

use crate::installer::run_installer;
use crate::system::{check_required_commands, load_disks, preflight_checks};
use crate::tui::{App, TuiSession};
use crate::types::FinalAction;

fn main() {
    if let Err(err) = run() {
        eprintln!("\nERROR: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    preflight_checks()?;
    check_required_commands()?;

    let disks = load_disks()?;
    if disks.is_empty() {
        bail!("No writable disks were found via lsblk.");
    }

    let mut app = App::new(disks);
    let mut term = TuiSession::start()?;
    let action = app.run_tui(term.terminal_mut())?;
    term.stop()?;

    match action {
        FinalAction::Exit => {
            println!("Aborted by user.");
            Ok(())
        }
        FinalAction::Install(cfg) => {
            println!("=== NixOS ZFS Installer (Rust) ===");
            println!("Target disk: {}", cfg.disk.path);
            println!("Boot size:   {}", cfg.boot_size);
            println!("Swap size:   {}", cfg.swap_size);
            println!("Hostname:    {}", cfg.hostname);
            println!("Timezone:    {}", cfg.timezone);
            println!("Keyboard:    {}", cfg.keyboard_layout);
            println!("User:        {}", cfg.username);
            println!(
                "Encryption:  {}",
                if cfg.enable_encryption { "enabled" } else { "disabled" }
            );
            println!(
                "Flakes:      {}",
                if cfg.enable_flakes { "enabled" } else { "disabled" }
            );
            println!(
                "Sudo:        {}",
                if cfg.enable_sudo { "enabled" } else { "disabled" }
            );
            println!(
                "NetworkMgr:  {}",
                if cfg.install_networkmanager {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            println!(
                "Git:         {}",
                if cfg.enable_git { "enabled" } else { "disabled" }
            );
            println!(
                "ZFS profile: {}",
                if cfg.zfs_use_recommended {
                    "recommended"
                } else {
                    "advanced"
                }
            );
            println!("ZFS ashift:  {}", cfg.zfs_ashift);
            println!("ZFS redun.:  {}", cfg.zfs_redundancy);
            println!("ZFS compr.:  {}", cfg.zfs_compression);
            println!("ZFS cache:   {}", cfg.zfs_primarycache);
            println!(
                "ZFS trim:    {}",
                if cfg.zfs_autotrim { "on" } else { "off" }
            );
            println!();
            run_installer(&cfg)
        }
    }
}
