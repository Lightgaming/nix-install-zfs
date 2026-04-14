#[derive(Clone, Debug)]
pub struct Disk {
    pub path: String,
    pub size: String,
    pub model: String,
}

#[derive(Clone)]
pub struct InstallConfig {
    pub disk: Disk,
    pub boot_size: String,
    pub swap_size: String,
    pub passphrase: String,
    pub enable_encryption: bool,
    pub enable_flakes: bool,
    pub hostname: String,
    pub timezone: String,
    pub keyboard_layout: String,
    pub username: String,
    pub user_password: String,
    pub enable_sudo: bool,
    pub root_password: String,
    pub install_networkmanager: bool,
    pub enable_git: bool,
    pub zfs_use_recommended: bool,
    pub zfs_ashift: String,
    pub zfs_redundancy: String,
    pub zfs_compression: String,
    pub zfs_primarycache: String,
    pub zfs_autotrim: bool,
}

pub enum FinalAction {
    Exit,
    Install(InstallConfig),
}
