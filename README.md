# nix-install-zfs

A guided NixOS ZFS installer written in Rust with a Ratatui terminal UI.

It replaces the old interactive shell flow with a safer TUI that helps reduce accidental data loss by adding:

- explicit disk selection from detected block devices
- guided one-by-one setup screens (not all fields on one page)
- validation for partition sizes and passphrase input
- explicit yes/no prompts for ZFS encryption and flakes enablement
- ZFS tuning wizard with Recommended defaults or Advanced mode
- guided prompts for system settings (hostname, timezone, keyboard layout)
- guided prompts for account setup (user, user password, sudo access, root password)
- guided prompts for optional components (NetworkManager, Git, SSH)
- warnings for mounted/system disks
- staged confirmations before destructive actions

## What it does

The installer performs the following high-level steps:

1. runs preflight checks (root, Linux, UEFI, required tools)
2. partitions the selected disk into EFI, swap, and ZFS root
3. formats EFI/swap and enables swap
4. creates ZFS pool `zroot` and datasets (encrypted or unencrypted based on your choice):
   - `zroot/root`
   - `zroot/nix`
   - `zroot/home`
   - `zroot/tmp`
5. mounts filesystems under `/mnt`
6. runs `nixos-generate-config --root /mnt`
7. writes `/mnt/etc/nixos/zfs.nix`
8. injects `./zfs.nix` into generated `configuration.nix`
9. writes `./system-setup.nix` with your selected system/user/network/program options

## ZFS configuration modes

The wizard provides two ZFS setup modes:

- Recommended defaults:
  - `ashift=12`
  - `redundancy=single`
  - `compression=lz4`
  - `primarycache=all`
  - `autotrim=on`
- Advanced mode:
  - `ashift`: `9..16`
  - `redundancy`: `single`, `mirror`, `raidz1`, `raidz2`, `raidz3`
  - `compression`: `lz4`, `zstd`, `gzip`, `zle`, `on`, `off`
  - `primarycache`: `all`, `metadata`, `none`
  - `autotrim`: yes/no

Note: current installer execution mode supports one target disk only, so only `redundancy=single` is executable right now. Other redundancy levels are shown and explained in Advanced mode, but require multi-disk pool support.

## Prerequisites

Run this from a NixOS installer/live environment with UEFI boot and root privileges.

This installer is set up for `systemd-boot` on UEFI systems.

- It enables `boot.loader.systemd-boot.enable = true`
- It disables GRUB (`boot.loader.grub.enable = false`)
- It expects UEFI firmware and checks for `/sys/firmware/efi`

If you need a BIOS/legacy-boot or GRUB-based setup, adjust the generated NixOS config before running `nixos-install`.

The binary expects these tools in `PATH`:

- `lsblk`, `findmnt`, `readlink`
- `wipefs`, `sgdisk`, `partprobe`, `udevadm`
- `mkfs.fat`, `mkswap`, `swapon`, `swapoff`
- `mount`, `umount`
- `zpool`, `zfs`, `blkid`
- `nixos-generate-config`

When launched with `nix run .`, the flake wrapper provides required runtime tools.

## Testing in a VM

Testing in a VM is strongly recommended before using this on real hardware.

Create a VM with at least:

- 2 vCPU
- 4 GB RAM (8 GB preferred)
- 40+ GB virtual disk
- NixOS ISO attached as optical media
- UEFI firmware enabled

### VirtualBox (UEFI)

1. Create a new VM (Linux, Other Linux 64-bit is fine).
2. Open VM settings.
3. Go to `System -> Motherboard` and enable `EFI (special OSes only)`.
4. Optional: disable `Floppy` in boot order to avoid odd boot behavior.
5. Attach the NixOS ISO under `Storage`.
6. Boot the VM and verify UEFI mode inside the live system:

```bash
test -d /sys/firmware/efi && echo UEFI || echo Legacy
```

### VMware Workstation / Player (UEFI)

1. Create a new VM and attach the NixOS ISO.
2. Open VM settings and confirm disk/controller resources.
3. Enable UEFI firmware:

- Workstation UI: `VM Settings -> Options -> Advanced -> Firmware type -> UEFI`
- If UI option is unavailable, power off VM and add this to the `.vmx` file:

```text
firmware = "efi"
```

4. Boot the VM and verify UEFI mode:

```bash
test -d /sys/firmware/efi && echo UEFI || echo Legacy
```

Then run the installer command from this README inside the live NixOS shell.

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
- Wizard steps: each page asks one value at a time (`Boot size`, `Swap size`, `Encryption`, `Passphrase`, `Flakes`)
- ZFS pages include `Recommended vs Advanced` and, in advanced mode, `ashift`, `redundancy`, `compression`, `primarycache`, and `autotrim`
- Additional guided pages configure hostname, timezone, keyboard layout, users, root password, network, git, and ssh
- Value entry pages: typing, `Backspace`, `Enter` next, `Esc` back
- Yes/No pages (encryption, flakes): arrow keys + `Enter`
- Existing configuration prompt: arrow keys + `Enter` (or `O` / `K`)
- Final confirmation: arrow keys + `Enter`, `F10` quick proceed, `Esc` to cancel

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
