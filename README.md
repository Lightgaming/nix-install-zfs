# nix-install-zfs

A guided NixOS ZFS installer written in Rust with a Ratatui terminal UI.

It replaces the old interactive shell flow with a safer TUI that helps reduce accidental data loss by adding:

- explicit disk selection from detected block devices
- validation for partition sizes and passphrase input
- warnings for mounted/system disks
- staged confirmations before destructive actions

## What it does

The installer performs the following high-level steps:

1. runs preflight checks (root, Linux, UEFI, required tools)
2. partitions the selected disk into EFI, swap, and ZFS root
3. formats EFI/swap and enables swap
4. creates encrypted ZFS pool `zroot` and datasets:
   - `zroot/root`
   - `zroot/nix`
   - `zroot/home`
   - `zroot/tmp`
5. mounts filesystems under `/mnt`
6. runs `nixos-generate-config --root /mnt`
7. writes `/mnt/etc/nixos/zfs.nix`
8. injects `./zfs.nix` into generated `configuration.nix`

## Prerequisites

Run this from a NixOS installer/live environment with UEFI boot and root privileges.

The binary expects these tools in `PATH`:

- `lsblk`, `findmnt`, `readlink`
- `wipefs`, `sgdisk`, `partprobe`, `udevadm`
- `mkfs.fat`, `mkswap`, `swapon`, `swapoff`
- `mount`, `umount`
- `zpool`, `zfs`, `blkid`
- `nixos-generate-config`

When launched with `nix run .`, the flake wrapper provides required runtime tools.

## Usage

### Run with Nix

```bash
nix run .
```

Or run directly from GitHub without cloning:

```bash
sudo nix run --extra-experimental-features nix-command --extra-experimental-features flakes --no-write-lock-file github:Lightgaming/nix-install-zfs
```

### Run with Cargo (development)

```bash
cargo run
```

## TUI controls

- Disk selection: `Up` / `Down`, `Enter`
- Form editing: `Tab`, `Shift+Tab`, typing, `Backspace`, `Enter`
- Existing configuration prompt: `O` (overwrite) or `K` (keep/exit)
- Final confirmation: `F10` to proceed, `Esc` to cancel

## After success

Review generated config:

```bash
nano /mnt/etc/nixos/configuration.nix
```

Install NixOS:

```bash
nixos-install
```

## Safety note

This tool performs destructive disk operations and will erase the selected disk.
Always verify the selected device before confirming with `F10`.
