#!/usr/bin/env bash

# Exit immediately if a command exits with a non-zero status
set -euo pipefail

# --- Pre-flight checks ---
if [ "$EUID" -ne 0 ]; then
  echo -e "\e[31mPlease run this script as root (e.g., sudo nix run github:...). \e[0m"
  exit 1
fi

if [ ! -d "/sys/firmware/efi" ]; then
  echo -e "\e[31mWARNING: You are not booted in UEFI mode! systemd-boot requires UEFI.\e[0m"
  echo "Please restart the ISO and boot in UEFI mode."
  exit 1
fi

echo -e "\e[1;36m=== NixOS ZFS Auto-Installer ===\e[0m\n"

# --- 1. User Inputs ---
lsblk -dpno NAME,SIZE,MODEL | grep -v "loop"
echo ""
read -p "Enter target disk (e.g., /dev/nvme0n1 or /dev/sda): " DISK

if [ ! -b "$DISK" ]; then
    echo -e "\e[31mError: $DISK is not a valid block device.\e[0m"
    exit 1
fi

read -p "Enter Boot partition size (default 1G): " BOOT_SIZE
BOOT_SIZE=${BOOT_SIZE:-1G}

read -p "Enter Swap partition size (default 8G): " SWAP_SIZE
SWAP_SIZE=${SWAP_SIZE:-8G}

read -s -p "Enter ZFS encryption passphrase: " ZFS_PASS
echo ""
read -s -p "Confirm ZFS encryption passphrase: " ZFS_PASS2
echo ""
if [ "$ZFS_PASS" != "$ZFS_PASS2" ]; then 
    echo -e "\e[31mPasswords do not match! Exiting.\e[0m"
    exit 1 
fi

# --- 2. Check for Existing Configurations ---
# If our labels exist on the disk, or zroot is active, we intercept.
HAS_EXISTING=false
if lsblk -o PARTLABEL -n "$DISK" 2>/dev/null | grep -q "NIXROOT"; then
    HAS_EXISTING=true
elif zpool status zroot >/dev/null 2>&1; then
    HAS_EXISTING=true
fi

if [ "$HAS_EXISTING" = true ]; then
    echo -e "\n\e[1;33m[!] Existing configuration detected on $DISK [!]\e[0m"
    
    # Try to read the old sizes for comparison
    OLD_BOOT=$(lsblk -b -n -o SIZE /dev/disk/by-partlabel/NIXBOOT 2>/dev/null | head -n1 | awk '{ printf "%.1fG", $1/1024/1024/1024 }' || echo "Unknown")
    OLD_SWAP=$(lsblk -b -n -o SIZE /dev/disk/by-partlabel/NIXSWAP 2>/dev/null | head -n1 | awk '{ printf "%.1fG", $1/1024/1024/1024 }' || echo "Unknown")
    
    echo "Current partitions on disk : Boot (~$OLD_BOOT), Swap (~$OLD_SWAP), ZFS Root"
    echo "Requested configuration    : Boot ($BOOT_SIZE), Swap ($SWAP_SIZE), ZFS Root"
    echo ""
    echo "If the configuration is already correct, you can KEEP it."
    read -p "Do you want to (O)verwrite and wipe it, or (K)eep existing and exit? [O/K]: " OVERWRITE_CHOICE
    
    if [[ "${OVERWRITE_CHOICE,,}" != "o" ]]; then
        echo -e "\n\e[1;32mKeeping existing configuration. You can now run:\e[0m"
        echo "  nixos-install"
        exit 0
    fi
fi

# --- 3. Confirmation Prompt ---
echo ""
echo -e "\e[1;41m WARNING \e[0m\e[31m THIS WILL COMPLETELY ERASE ALL DATA ON \e[1m$DISK\e[0m"
read -p "Type 'yes' to proceed: " CONFIRM

if [ "$CONFIRM" != "yes" ]; then
    echo "Aborted by user."
    exit 1
fi

# --- 4. Cleanup Function (Prevents "Device or resource busy") ---
echo -e "\n\e[1;34m--> Cleaning up existing mounts and locks...\e[0m"
# Unmount any residual filesystems
if grep -q /mnt /proc/mounts; then
    umount -R /mnt 2>/dev/null || true
fi
# Turn off swap globally (safest on a live USB)
swapoff -a 2>/dev/null || true
# Destroy the pool if it exists so it releases the disk
if zpool list zroot >/dev/null 2>&1; then
    zpool destroy -f zroot 2>/dev/null || true
fi
# Give the kernel 2 seconds to release the file descriptors
sleep 2 

# --- 5. Wipe and Partition ---
echo -e "\e[1;34m--> Wiping disk and creating partitions...\e[0m"
wipefs -a "$DISK"
sgdisk --zap-all "$DISK"

# Create partitions and label them
sgdisk -n 1:0:+${BOOT_SIZE} -t 1:ef00 -c 1:NIXBOOT "$DISK"
sgdisk -n 2:0:+${SWAP_SIZE} -t 2:8200 -c 2:NIXSWAP "$DISK"
sgdisk -n 3:0:0 -t 3:8300 -c 3:NIXROOT "$DISK"

partprobe "$DISK"
sleep 3 # Give udev a moment to populate /dev/disk/by-partlabel/

PART_BOOT="/dev/disk/by-partlabel/NIXBOOT"
PART_SWAP="/dev/disk/by-partlabel/NIXSWAP"
PART_ZFS="/dev/disk/by-partlabel/NIXROOT"

# --- 6. Format Non-ZFS Partitions ---
echo -e "\e[1;34m--> Formatting Boot and Swap...\e[0m"
mkfs.fat -F 32 -n NIXBOOT "$PART_BOOT"
mkswap -L SWAP "$PART_SWAP"
swapon "$PART_SWAP"

# --- 7. Create Encrypted ZFS Pool & Datasets ---
echo -e "\e[1;34m--> Creating Encrypted ZFS Pool...\e[0m"
echo "$ZFS_PASS" | zpool create -f \
    -o ashift=12 \
    -o autotrim=on \
    -O compression=lz4 \
    -O acltype=posixacl \
    -O atime=off \
    -O xattr=sa \
    -O normalization=formD \
    -O mountpoint=none \
    -O encryption=aes-256-gcm \
    -O keyformat=passphrase \
    -O keylocation=prompt \
    zroot "$PART_ZFS"

zfs create -o mountpoint=legacy zroot/root
zfs create -o mountpoint=legacy zroot/nix
zfs create -o mountpoint=legacy zroot/home
zfs create -o mountpoint=legacy zroot/tmp

# --- 8. Mount Filesystems ---
echo -e "\e[1;34m--> Mounting filesystems...\e[0m"
mount -t zfs zroot/root /mnt

mkdir -p /mnt/{boot,nix,home,tmp}
mount "$PART_BOOT" /mnt/boot
mount -t zfs zroot/nix /mnt/nix
mount -t zfs zroot/home /mnt/home
mount -t zfs zroot/tmp /mnt/tmp

# --- 9. Generate NixOS Configurations ---
echo -e "\e[1;34m--> Generating NixOS configuration...\e[0m"
nixos-generate-config --root /mnt

# --- 10. Inject ZFS Sub-module ---
echo -e "\e[1;34m--> Applying ZFS and Boot overrides...\e[0m"

HOSTID=$(od -A n -t x4 -N 4 /dev/urandom | tr -d ' \n')
SWAP_PARTUUID=$(blkid -s PARTUUID -o value "$PART_SWAP")

# Safely inject overrides via a standalone module to avoid syntax errors
cat <<EOF > /mnt/etc/nixos/zfs.nix
{ config, pkgs, ... }:

{
  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
  boot.loader.grub.enable = pkgs.lib.mkForce false;

  networking.hostId = "${HOSTID}";
  boot.supportedFilesystems = [ "zfs" ];

  swapDevices = pkgs.lib.mkForce [ {
    device = "/dev/disk/by-partuuid/${SWAP_PARTUUID}";
    randomEncryption.enable = true;
  } ];
}
EOF

# Wire the sub-module into configuration.nix
sed -i 's|\./hardware-configuration\.nix|\./hardware-configuration.nix ./zfs.nix|' /mnt/etc/nixos/configuration.nix

# --- 11. Hand off to User ---
echo -e "\n\e[1;32m=== ZFS Configuration Complete! ===\e[0m"
echo -e "Your disks have been formatted, mounted, and your configuration files"
echo -e "at \e[1m/mnt/etc/nixos/\e[0m are ready."
echo -e "\nTo review the main config before installing, run:"
echo -e "  nano /mnt/etc/nixos/configuration.nix"
echo -e "\nTo proceed with the installation, simply run:"
echo -e "  \e[1;33mnixos-install\e[0m\n"