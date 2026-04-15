use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear as TermClear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::system::{
    collect_disk_warnings, has_existing_configuration, validate_hostname, validate_keyboard_layout,
    validate_password, validate_size_input, validate_timezone, validate_username,
    validate_zfs_ashift, validate_zfs_compression, validate_zfs_primarycache,
    validate_zfs_redundancy,
};
use crate::types::{Disk, FinalAction, InstallConfig};

#[derive(Clone, Copy, PartialEq, Eq)]
enum UiScreen {
    DiskSelect,
    BootSize,
    SwapSize,
    EncryptionChoice,
    Passphrase,
    PassphraseConfirm,
    FlakesChoice,
    ZfsModeChoice,
    ZfsAshift,
    ZfsRedundancy,
    ZfsCompression,
    ZfsCaching,
    ZfsAutotrimChoice,
    Hostname,
    Timezone,
    KeyboardLayout,
    Username,
    UserPassword,
    UserPasswordConfirm,
    EnableSudoChoice,
    RootPassword,
    RootPasswordConfirm,
    NetworkManagerChoice,
    FeatureToggles,
    ExistingConfirm,
    FinalConfirm,
}

pub struct App {
    disks: Vec<Disk>,
    selected_disk: usize,
    screen: UiScreen,
    list_state: ListState,
    boot_size: String,
    swap_size: String,
    enable_encryption: bool,
    passphrase: String,
    passphrase_confirm: String,
    enable_flakes: bool,
    hostname: String,
    timezone: String,
    keyboard_layout: String,
    username: String,
    user_password: String,
    user_password_confirm: String,
    enable_sudo: bool,
    root_password: String,
    root_password_confirm: String,
    install_networkmanager: bool,
    enable_git: bool,
    enable_ssh: bool,
    feature_toggle_selected: usize,
    zfs_use_recommended: bool,
    zfs_ashift: String,
    zfs_redundancy: String,
    zfs_compression: String,
    zfs_primarycache: String,
    zfs_autotrim: bool,
    existing_found: bool,
    choice_yes_selected: bool,
    existing_overwrite_selected: bool,
    final_proceed_selected: bool,
    warnings: Vec<String>,
    status: String,
}

impl App {
    pub fn new(disks: Vec<Disk>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            disks,
            selected_disk: 0,
            screen: UiScreen::DiskSelect,
            list_state,
            boot_size: "1G".to_string(),
            swap_size: "8G".to_string(),
            enable_encryption: true,
            passphrase: String::new(),
            passphrase_confirm: String::new(),
            enable_flakes: true,
            hostname: "nixos".to_string(),
            timezone: "UTC".to_string(),
            keyboard_layout: "us".to_string(),
            username: "nixos".to_string(),
            user_password: String::new(),
            user_password_confirm: String::new(),
            enable_sudo: true,
            root_password: String::new(),
            root_password_confirm: String::new(),
            install_networkmanager: true,
            enable_git: true,
            enable_ssh: false,
            feature_toggle_selected: 0,
            zfs_use_recommended: true,
            zfs_ashift: "12".to_string(),
            zfs_redundancy: "single".to_string(),
            zfs_compression: "lz4".to_string(),
            zfs_primarycache: "all".to_string(),
            zfs_autotrim: true,
            existing_found: false,
            choice_yes_selected: true,
            existing_overwrite_selected: true,
            final_proceed_selected: true,
            warnings: Vec::new(),
            status: "Select installation disk with arrow keys, Enter to continue.".to_string(),
        }
    }

    pub fn run_tui(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<FinalAction> {
        loop {
            terminal.draw(|f| self.draw(f))?;

            if event::poll(std::time::Duration::from_millis(150))? {
                let ev = event::read()?;
                if let Event::Key(key) = ev {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    if let Some(action) = self.handle_key(key.code)? {
                        return Ok(action);
                    }
                }
            }
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match self.screen {
            UiScreen::DiskSelect => self.handle_disk_key(code),
            UiScreen::BootSize => self.handle_boot_size_key(code),
            UiScreen::SwapSize => self.handle_swap_size_key(code),
            UiScreen::EncryptionChoice => self.handle_encryption_choice_key(code),
            UiScreen::Passphrase => self.handle_passphrase_key(code),
            UiScreen::PassphraseConfirm => self.handle_passphrase_confirm_key(code),
            UiScreen::FlakesChoice => self.handle_flakes_choice_key(code),
            UiScreen::ZfsModeChoice => self.handle_zfs_mode_choice_key(code),
            UiScreen::ZfsAshift => self.handle_zfs_ashift_key(code),
            UiScreen::ZfsRedundancy => self.handle_zfs_redundancy_key(code),
            UiScreen::ZfsCompression => self.handle_zfs_compression_key(code),
            UiScreen::ZfsCaching => self.handle_zfs_caching_key(code),
            UiScreen::ZfsAutotrimChoice => self.handle_zfs_autotrim_key(code),
            UiScreen::Hostname => self.handle_hostname_key(code),
            UiScreen::Timezone => self.handle_timezone_key(code),
            UiScreen::KeyboardLayout => self.handle_keyboard_layout_key(code),
            UiScreen::Username => self.handle_username_key(code),
            UiScreen::UserPassword => self.handle_user_password_key(code),
            UiScreen::UserPasswordConfirm => self.handle_user_password_confirm_key(code),
            UiScreen::EnableSudoChoice => self.handle_enable_sudo_key(code),
            UiScreen::RootPassword => self.handle_root_password_key(code),
            UiScreen::RootPasswordConfirm => self.handle_root_password_confirm_key(code),
            UiScreen::NetworkManagerChoice => self.handle_networkmanager_key(code),
            UiScreen::FeatureToggles => self.handle_feature_toggles_key(code),
            UiScreen::ExistingConfirm => self.handle_existing_key(code),
            UiScreen::FinalConfirm => self.handle_final_key(code),
        }
    }

    fn handle_disk_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(Some(FinalAction::Exit)),
            KeyCode::Down => {
                if self.selected_disk + 1 < self.disks.len() {
                    self.selected_disk += 1;
                }
            }
            KeyCode::Up => {
                if self.selected_disk > 0 {
                    self.selected_disk -= 1;
                }
            }
            KeyCode::Enter => {
                self.list_state.select(Some(self.selected_disk));
                self.screen = UiScreen::BootSize;
                self.status = "Step 2/7: Set EFI boot partition size, then press Enter.".to_string();
            }
            _ => {}
        }
        self.list_state.select(Some(self.selected_disk));
        Ok(None)
    }

    fn handle_boot_size_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::DiskSelect;
                self.status = "Select installation disk with arrow keys, Enter to continue.".to_string();
            }
            KeyCode::Backspace => {
                self.boot_size.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.boot_size.push(c.to_ascii_uppercase());
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_size_input(&self.boot_size, "Boot") {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::SwapSize;
                    self.status = "Step 3/7: Set swap partition size, then press Enter.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_swap_size_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::BootSize;
                self.status = "Step 2/7: Set EFI boot partition size, then press Enter.".to_string();
            }
            KeyCode::Backspace => {
                self.swap_size.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.swap_size.push(c.to_ascii_uppercase());
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_size_input(&self.swap_size, "Swap") {
                    self.status = err.to_string();
                } else {
                    self.choice_yes_selected = self.enable_encryption;
                    self.screen = UiScreen::EncryptionChoice;
                    self.status =
                        "Step 4/7: Choose whether to enable ZFS encryption.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_encryption_choice_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::SwapSize;
                self.status = "Step 3/7: Set swap partition size, then press Enter.".to_string();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.choice_yes_selected = !self.choice_yes_selected;
            }
            KeyCode::Enter => {
                self.enable_encryption = self.choice_yes_selected;
                if self.enable_encryption {
                    self.passphrase.clear();
                    self.passphrase_confirm.clear();
                    self.screen = UiScreen::Passphrase;
                    self.status = "Step 5/7: Enter encryption passphrase (min 8 chars).".to_string();
                } else {
                    self.choice_yes_selected = self.enable_flakes;
                    self.screen = UiScreen::FlakesChoice;
                    self.status = "Step 7/7: Choose whether to enable flakes by default.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_passphrase_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.choice_yes_selected = self.enable_encryption;
                self.screen = UiScreen::EncryptionChoice;
                self.status =
                    "Step 4/7: Choose whether to enable ZFS encryption.".to_string();
            }
            KeyCode::Backspace => {
                self.passphrase.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.passphrase.push(c);
                }
            }
            KeyCode::Enter => {
                if self.passphrase.len() < 8 {
                    self.status = "Passphrase must be at least 8 characters.".to_string();
                } else {
                    self.screen = UiScreen::PassphraseConfirm;
                    self.status = "Step 6/7: Confirm encryption passphrase.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_passphrase_confirm_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::Passphrase;
                self.status = "Step 5/7: Enter encryption passphrase (min 8 chars).".to_string();
            }
            KeyCode::Backspace => {
                self.passphrase_confirm.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.passphrase_confirm.push(c);
                }
            }
            KeyCode::Enter => {
                if self.passphrase != self.passphrase_confirm {
                    self.status = "Passphrase and confirmation do not match.".to_string();
                } else {
                    self.choice_yes_selected = self.enable_flakes;
                    self.screen = UiScreen::FlakesChoice;
                    self.status = "Step 7/7: Choose whether to enable flakes by default.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_flakes_choice_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                if self.enable_encryption {
                    self.screen = UiScreen::PassphraseConfirm;
                    self.status = "Step 6/7: Confirm encryption passphrase.".to_string();
                } else {
                    self.choice_yes_selected = self.enable_encryption;
                    self.screen = UiScreen::EncryptionChoice;
                    self.status =
                        "Step 4/7: Choose whether to enable ZFS encryption.".to_string();
                }
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.choice_yes_selected = !self.choice_yes_selected;
            }
            KeyCode::Enter => {
                self.enable_flakes = self.choice_yes_selected;
                self.choice_yes_selected = self.zfs_use_recommended;
                self.screen = UiScreen::ZfsModeChoice;
                self.status =
                    "ZFS setup: choose Recommended defaults or Advanced configuration."
                        .to_string();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_zfs_mode_choice_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.choice_yes_selected = self.enable_flakes;
                self.screen = UiScreen::FlakesChoice;
                self.status = "Step 7/7: Choose whether to enable flakes by default.".to_string();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.choice_yes_selected = !self.choice_yes_selected;
            }
            KeyCode::Enter => {
                self.zfs_use_recommended = self.choice_yes_selected;
                if self.zfs_use_recommended {
                    self.zfs_ashift = "12".to_string();
                    self.zfs_redundancy = "single".to_string();
                    self.zfs_compression = "lz4".to_string();
                    self.zfs_primarycache = "all".to_string();
                    self.zfs_autotrim = true;
                    self.screen = UiScreen::Hostname;
                    self.status = "Next: set hostname, then press Enter.".to_string();
                } else {
                    self.screen = UiScreen::ZfsAshift;
                    self.status =
                        "Advanced ZFS: set ashift (recommended 12 for 4K sector disks)."
                            .to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_zfs_ashift_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.choice_yes_selected = self.zfs_use_recommended;
                self.screen = UiScreen::ZfsModeChoice;
                self.status =
                    "ZFS setup: choose Recommended defaults or Advanced configuration."
                        .to_string();
            }
            KeyCode::Backspace => {
                self.zfs_ashift.pop();
            }
            KeyCode::Char(c) => {
                if c.is_ascii_digit() {
                    self.zfs_ashift.push(c);
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_zfs_ashift(&self.zfs_ashift) {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::ZfsRedundancy;
                    self.status =
                        "Advanced ZFS: set redundancy (single, mirror, raidz1, raidz2, raidz3)."
                            .to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_zfs_redundancy_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::ZfsAshift;
                self.status =
                    "Advanced ZFS: set ashift (recommended 12 for 4K sector disks)."
                        .to_string();
            }
            KeyCode::Backspace => {
                self.zfs_redundancy.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.zfs_redundancy.push(c.to_ascii_lowercase());
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_zfs_redundancy(&self.zfs_redundancy) {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::ZfsCompression;
                    self.status =
                        "Advanced ZFS: set compression (lz4, zstd, gzip, zle, on, off)."
                            .to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_zfs_compression_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::ZfsRedundancy;
                self.status =
                    "Advanced ZFS: set redundancy (single, mirror, raidz1, raidz2, raidz3)."
                        .to_string();
            }
            KeyCode::Backspace => {
                self.zfs_compression.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.zfs_compression.push(c.to_ascii_lowercase());
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_zfs_compression(&self.zfs_compression) {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::ZfsCaching;
                    self.status =
                        "Advanced ZFS: set primarycache (all, metadata, none).".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_zfs_caching_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::ZfsCompression;
                self.status =
                    "Advanced ZFS: set compression (lz4, zstd, gzip, zle, on, off)."
                        .to_string();
            }
            KeyCode::Backspace => {
                self.zfs_primarycache.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.zfs_primarycache.push(c.to_ascii_lowercase());
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_zfs_primarycache(&self.zfs_primarycache) {
                    self.status = err.to_string();
                } else {
                    self.choice_yes_selected = self.zfs_autotrim;
                    self.screen = UiScreen::ZfsAutotrimChoice;
                    self.status =
                        "Advanced ZFS: choose whether to enable autotrim for SSD/NVMe.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_zfs_autotrim_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::ZfsCaching;
                self.status = "Advanced ZFS: set primarycache (all, metadata, none).".to_string();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.choice_yes_selected = !self.choice_yes_selected;
            }
            KeyCode::Enter => {
                self.zfs_autotrim = self.choice_yes_selected;
                self.screen = UiScreen::Hostname;
                self.status = "Next: set hostname, then press Enter.".to_string();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_hostname_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                if self.zfs_use_recommended {
                    self.choice_yes_selected = self.zfs_use_recommended;
                    self.screen = UiScreen::ZfsModeChoice;
                    self.status =
                        "ZFS setup: choose Recommended defaults or Advanced configuration."
                            .to_string();
                } else {
                    self.choice_yes_selected = self.zfs_autotrim;
                    self.screen = UiScreen::ZfsAutotrimChoice;
                    self.status =
                        "Advanced ZFS: choose whether to enable autotrim for SSD/NVMe."
                            .to_string();
                }
            }
            KeyCode::Backspace => {
                self.hostname.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.hostname.push(c);
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_hostname(&self.hostname) {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::Timezone;
                    self.status = "Next: set timezone (e.g. Europe/Berlin), then press Enter.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_timezone_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::Hostname;
                self.status = "Next: set hostname, then press Enter.".to_string();
            }
            KeyCode::Backspace => {
                self.timezone.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.timezone.push(c);
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_timezone(&self.timezone) {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::KeyboardLayout;
                    self.status = "Next: set keyboard layout (e.g. us, de), then press Enter.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_keyboard_layout_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::Timezone;
                self.status = "Next: set timezone, then press Enter.".to_string();
            }
            KeyCode::Backspace => {
                self.keyboard_layout.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.keyboard_layout.push(c);
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_keyboard_layout(&self.keyboard_layout) {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::Username;
                    self.status = "Next: set username, then press Enter.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_username_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::KeyboardLayout;
                self.status = "Next: set keyboard layout, then press Enter.".to_string();
            }
            KeyCode::Backspace => {
                self.username.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.username.push(c);
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_username(&self.username) {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::UserPassword;
                    self.status = "Next: enter user password, then press Enter.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_user_password_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::Username;
                self.status = "Next: set username, then press Enter.".to_string();
            }
            KeyCode::Backspace => {
                self.user_password.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.user_password.push(c);
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_password(&self.user_password, "User password") {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::UserPasswordConfirm;
                    self.status = "Next: confirm user password, then press Enter.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_user_password_confirm_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::UserPassword;
                self.status = "Next: enter user password, then press Enter.".to_string();
            }
            KeyCode::Backspace => {
                self.user_password_confirm.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.user_password_confirm.push(c);
                }
            }
            KeyCode::Enter => {
                if self.user_password != self.user_password_confirm {
                    self.status = "User password and confirmation do not match.".to_string();
                } else {
                    self.choice_yes_selected = self.enable_sudo;
                    self.screen = UiScreen::EnableSudoChoice;
                    self.status = "Next: choose whether the user should have sudo access.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_enable_sudo_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::UserPasswordConfirm;
                self.status = "Next: confirm user password, then press Enter.".to_string();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.choice_yes_selected = !self.choice_yes_selected;
            }
            KeyCode::Enter => {
                self.enable_sudo = self.choice_yes_selected;
                self.screen = UiScreen::RootPassword;
                self.status = "Next: enter root password, then press Enter.".to_string();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_root_password_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.choice_yes_selected = self.enable_sudo;
                self.screen = UiScreen::EnableSudoChoice;
                self.status = "Next: choose whether the user should have sudo access.".to_string();
            }
            KeyCode::Backspace => {
                self.root_password.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.root_password.push(c);
                }
            }
            KeyCode::Enter => {
                if let Err(err) = validate_password(&self.root_password, "Root password") {
                    self.status = err.to_string();
                } else {
                    self.screen = UiScreen::RootPasswordConfirm;
                    self.status = "Next: confirm root password, then press Enter.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_root_password_confirm_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::RootPassword;
                self.status = "Next: enter root password, then press Enter.".to_string();
            }
            KeyCode::Backspace => {
                self.root_password_confirm.pop();
            }
            KeyCode::Char(c) => {
                if !c.is_control() {
                    self.root_password_confirm.push(c);
                }
            }
            KeyCode::Enter => {
                if self.root_password != self.root_password_confirm {
                    self.status = "Root password and confirmation do not match.".to_string();
                } else {
                    self.choice_yes_selected = self.install_networkmanager;
                    self.screen = UiScreen::NetworkManagerChoice;
                    self.status = "Next: choose whether to enable NetworkManager.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_networkmanager_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.screen = UiScreen::RootPasswordConfirm;
                self.status = "Next: confirm root password, then press Enter.".to_string();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.choice_yes_selected = !self.choice_yes_selected;
            }
            KeyCode::Enter => {
                self.install_networkmanager = self.choice_yes_selected;
                self.feature_toggle_selected = 0;
                self.screen = UiScreen::FeatureToggles;
                self.status =
                    "Next: use Up/Down to pick Git or SSH, toggle with Enter/Space, then Continue."
                        .to_string();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_feature_toggles_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => {
                self.choice_yes_selected = self.install_networkmanager;
                self.screen = UiScreen::NetworkManagerChoice;
                self.status = "Next: choose whether to enable NetworkManager.".to_string();
            }
            KeyCode::Up => {
                if self.feature_toggle_selected > 0 {
                    self.feature_toggle_selected -= 1;
                }
            }
            KeyCode::Down => {
                if self.feature_toggle_selected < 2 {
                    self.feature_toggle_selected += 1;
                }
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::Char(' ') => {
                match self.feature_toggle_selected {
                    0 => self.enable_git = !self.enable_git,
                    1 => self.enable_ssh = !self.enable_ssh,
                    _ => {}
                }
            }
            KeyCode::Enter => {
                match self.feature_toggle_selected {
                    0 => {
                        self.enable_git = !self.enable_git;
                    }
                    1 => {
                        self.enable_ssh = !self.enable_ssh;
                    }
                    _ => {
                        self.warnings = collect_disk_warnings(&self.disks[self.selected_disk].path)?;
                        self.existing_found =
                            has_existing_configuration(&self.disks[self.selected_disk].path)?;
                        if self.existing_found {
                            self.existing_overwrite_selected = true;
                            self.screen = UiScreen::ExistingConfirm;
                            self.status =
                                "Existing install markers detected. Use arrows, then Enter to choose."
                                    .to_string();
                        } else {
                            self.final_proceed_selected = true;
                            self.screen = UiScreen::FinalConfirm;
                            self.status =
                                "Final confirmation. Use arrows and Enter (or F10) to continue."
                                    .to_string();
                        }
                    }
                }
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                self.feature_toggle_selected = 2;
                self.warnings = collect_disk_warnings(&self.disks[self.selected_disk].path)?;
                self.existing_found = has_existing_configuration(&self.disks[self.selected_disk].path)?;
                if self.existing_found {
                    self.existing_overwrite_selected = true;
                    self.screen = UiScreen::ExistingConfirm;
                    self.status =
                        "Existing install markers detected. Use arrows, then Enter to choose."
                            .to_string();
                } else {
                    self.final_proceed_selected = true;
                    self.screen = UiScreen::FinalConfirm;
                    self.status =
                        "Final confirmation. Use arrows and Enter (or F10) to continue.".to_string();
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_existing_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => return Ok(Some(FinalAction::Exit)),
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.existing_overwrite_selected = !self.existing_overwrite_selected;
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                self.existing_overwrite_selected = true;
                self.final_proceed_selected = true;
                self.screen = UiScreen::FinalConfirm;
                self.status = "Overwrite confirmed. Use arrows and Enter (or F10) to continue."
                    .to_string();
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                self.existing_overwrite_selected = false;
                return Ok(Some(FinalAction::Exit));
            }
            KeyCode::Enter => {
                if self.existing_overwrite_selected {
                    self.final_proceed_selected = true;
                    self.screen = UiScreen::FinalConfirm;
                    self.status = "Overwrite confirmed. Use arrows and Enter (or F10) to continue."
                        .to_string();
                } else {
                    return Ok(Some(FinalAction::Exit));
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_final_key(&mut self, code: KeyCode) -> Result<Option<FinalAction>> {
        match code {
            KeyCode::Esc => return Ok(Some(FinalAction::Exit)),
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                self.final_proceed_selected = !self.final_proceed_selected;
            }
            KeyCode::Enter => {
                if !self.final_proceed_selected {
                    return Ok(Some(FinalAction::Exit));
                }
                let cfg = self.build_install_config();
                return Ok(Some(FinalAction::Install(cfg)));
            }
            KeyCode::F(10) => {
                let cfg = self.build_install_config();
                return Ok(Some(FinalAction::Install(cfg)));
            }
            _ => {}
        }
        Ok(None)
    }

    fn build_install_config(&self) -> InstallConfig {
        InstallConfig {
            disk: self.disks[self.selected_disk].clone(),
            boot_size: self.boot_size.clone(),
            swap_size: self.swap_size.clone(),
            passphrase: self.passphrase.clone(),
            enable_encryption: self.enable_encryption,
            enable_flakes: self.enable_flakes,
            hostname: self.hostname.clone(),
            timezone: self.timezone.clone(),
            keyboard_layout: self.keyboard_layout.clone(),
            username: self.username.clone(),
            user_password: self.user_password.clone(),
            enable_sudo: self.enable_sudo,
            root_password: self.root_password.clone(),
            install_networkmanager: self.install_networkmanager,
            enable_git: self.enable_git,
            enable_ssh: self.enable_ssh,
            zfs_use_recommended: self.zfs_use_recommended,
            zfs_ashift: self.zfs_ashift.clone(),
            zfs_redundancy: self.zfs_redundancy.clone(),
            zfs_compression: self.zfs_compression.clone(),
            zfs_primarycache: self.zfs_primarycache.clone(),
            zfs_autotrim: self.zfs_autotrim,
        }
    }

    fn draw(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(4),
            ])
            .split(area);

        let title = Paragraph::new("NixOS ZFS Installer")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightBlue))
                    .title("Installer"),
            )
            .style(Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD));
        f.render_widget(title, chunks[0]);

        match self.screen {
            UiScreen::DiskSelect => self.draw_disk_select(f, chunks[1]),
            UiScreen::BootSize => self.draw_value_step(
                f,
                chunks[1],
                "Step 2/7 - Boot Partition",
                "Enter boot partition size (examples: 512M, 1G, 2G).",
                &self.boot_size,
                false,
            ),
            UiScreen::SwapSize => self.draw_value_step(
                f,
                chunks[1],
                "Step 3/7 - Swap Partition",
                "Enter swap partition size (examples: 4G, 8G, 16G).",
                &self.swap_size,
                false,
            ),
            UiScreen::EncryptionChoice => self.draw_yes_no_step(
                f,
                chunks[1],
                "Step 4/7 - Encryption",
                "Enable ZFS root encryption?",
                self.choice_yes_selected,
            ),
            UiScreen::Passphrase => self.draw_value_step(
                f,
                chunks[1],
                "Step 5/7 - Encryption Passphrase",
                "Enter passphrase (minimum 8 characters).",
                &self.passphrase,
                true,
            ),
            UiScreen::PassphraseConfirm => self.draw_value_step(
                f,
                chunks[1],
                "Step 6/7 - Confirm Passphrase",
                "Enter the same passphrase again.",
                &self.passphrase_confirm,
                true,
            ),
            UiScreen::FlakesChoice => self.draw_yes_no_step(
                f,
                chunks[1],
                "Step 7/7 - Nix Flakes",
                "Enable nix-command and flakes in generated configuration?",
                self.choice_yes_selected,
            ),
            UiScreen::ZfsModeChoice => self.draw_yes_no_step(
                f,
                chunks[1],
                "ZFS - Recommended or Advanced",
                "Use recommended defaults? Yes = ashift=12, redundancy=single, compression=lz4, primarycache=all, autotrim=on. No = configure each option manually.",
                self.choice_yes_selected,
            ),
            UiScreen::ZfsAshift => self.draw_value_step(
                f,
                chunks[1],
                "ZFS - ashift",
                "Set ashift (9-16). Common values: 12 for 4K sectors, 13 for 8K sectors.",
                &self.zfs_ashift,
                false,
            ),
            UiScreen::ZfsRedundancy => self.draw_value_step(
                f,
                chunks[1],
                "ZFS - Redundancy Level",
                "Set redundancy: single, mirror, raidz1, raidz2, raidz3. Note: single-disk mode currently supports only single.",
                &self.zfs_redundancy,
                false,
            ),
            UiScreen::ZfsCompression => self.draw_value_step(
                f,
                chunks[1],
                "ZFS - Compression",
                "Set compression: lz4 (recommended), zstd, gzip, zle, on, off.",
                &self.zfs_compression,
                false,
            ),
            UiScreen::ZfsCaching => self.draw_value_step(
                f,
                chunks[1],
                "ZFS - Caching",
                "Set primarycache: all (default), metadata, or none.",
                &self.zfs_primarycache,
                false,
            ),
            UiScreen::ZfsAutotrimChoice => self.draw_yes_no_step(
                f,
                chunks[1],
                "ZFS - Autotrim",
                "Enable autotrim? Recommended for SSD/NVMe, usually disabled for HDD-only pools.",
                self.choice_yes_selected,
            ),
            UiScreen::Hostname => self.draw_value_step(
                f,
                chunks[1],
                "System - Hostname",
                "Enter hostname (example: nixos, laptop, server01).",
                &self.hostname,
                false,
            ),
            UiScreen::Timezone => self.draw_value_step(
                f,
                chunks[1],
                "System - Timezone",
                "Enter timezone (example: UTC, Europe/Berlin, America/New_York).",
                &self.timezone,
                false,
            ),
            UiScreen::KeyboardLayout => self.draw_value_step(
                f,
                chunks[1],
                "System - Keyboard Layout",
                "Enter keymap/layout (example: us, de, fr).",
                &self.keyboard_layout,
                false,
            ),
            UiScreen::Username => self.draw_value_step(
                f,
                chunks[1],
                "User - Username",
                "Enter username for the initial user account.",
                &self.username,
                false,
            ),
            UiScreen::UserPassword => self.draw_value_step(
                f,
                chunks[1],
                "User - Password",
                "Enter password for the initial user account.",
                &self.user_password,
                true,
            ),
            UiScreen::UserPasswordConfirm => self.draw_value_step(
                f,
                chunks[1],
                "User - Confirm Password",
                "Enter the same user password again.",
                &self.user_password_confirm,
                true,
            ),
            UiScreen::EnableSudoChoice => self.draw_yes_no_step(
                f,
                chunks[1],
                "User - Sudo Access",
                "Should this user be added to the wheel (sudo) group?",
                self.choice_yes_selected,
            ),
            UiScreen::RootPassword => self.draw_value_step(
                f,
                chunks[1],
                "Root - Password",
                "Enter root password.",
                &self.root_password,
                true,
            ),
            UiScreen::RootPasswordConfirm => self.draw_value_step(
                f,
                chunks[1],
                "Root - Confirm Password",
                "Enter the same root password again.",
                &self.root_password_confirm,
                true,
            ),
            UiScreen::NetworkManagerChoice => self.draw_yes_no_step(
                f,
                chunks[1],
                "Network",
                "Enable NetworkManager?",
                self.choice_yes_selected,
            ),
            UiScreen::FeatureToggles => self.draw_feature_toggles(f, chunks[1]),
            UiScreen::ExistingConfirm => self.draw_existing_confirm(f, chunks[1]),
            UiScreen::FinalConfirm => self.draw_final_confirm(f, chunks[1]),
        }

        let footer = Paragraph::new(self.status.clone())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue))
                    .title("Status"),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(footer, chunks[2]);
    }

    fn draw_disk_select(&mut self, f: &mut Frame<'_>, area: Rect) {
        let items: Vec<ListItem> = self
            .disks
            .iter()
            .map(|d| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{}", d.path), Style::default().fg(Color::Yellow)),
                    Span::raw("  "),
                    Span::raw(format!("{}  {}", d.size, d.model)),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightBlue))
                    .title("Available Disks"),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");
        f.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn draw_value_step(
        &self,
        f: &mut Frame<'_>,
        area: Rect,
        title: &str,
        prompt: &str,
        value: &str,
        secret: bool,
    ) {
        let rendered_value = if secret {
            "*".repeat(value.chars().count())
        } else {
            value.to_string()
        };
        let lines = vec![
            Line::from(Span::styled(prompt, Style::default().fg(Color::LightYellow))),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "> Value: ",
                    Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
                ),
                Span::styled(rendered_value, Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from("Type to edit, Backspace to delete, Enter next, Esc back"),
        ];

        let p = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightBlue))
                    .title(title),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    }

    fn draw_yes_no_step(
        &self,
        f: &mut Frame<'_>,
        area: Rect,
        title: &str,
        prompt: &str,
        yes_selected: bool,
    ) {
        let yes_style = if yes_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let no_style = if yes_selected {
            Style::default().fg(Color::Gray)
        } else {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightRed)
                .add_modifier(Modifier::BOLD)
        };

        let lines = vec![
            Line::from(Span::styled(prompt, Style::default().fg(Color::LightYellow))),
            Line::from(""),
            Line::from("Use Left/Right or Up/Down to choose, Enter to confirm."),
            Line::from(""),
            Line::from(vec![
                Span::styled("[ Yes ]", yes_style),
                Span::raw("   "),
                Span::styled("[ No ]", no_style),
            ]),
        ];

        let p = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightBlue))
                    .title(title),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    }

    fn draw_feature_toggles(&self, f: &mut Frame<'_>, area: Rect) {
        let selected_style = Style::default()
            .fg(Color::Black)
            .bg(Color::LightGreen)
            .add_modifier(Modifier::BOLD);

        let normal_style = Style::default().fg(Color::White);

        let rows = [
            format!(
                "Git: {}",
                if self.enable_git { "[ON]" } else { "[OFF]" }
            ),
            format!(
                "SSH: {}",
                if self.enable_ssh { "[ON]" } else { "[OFF]" }
            ),
            "Continue".to_string(),
        ];

        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .map(|(idx, row)| {
                let marker = if idx == self.feature_toggle_selected {
                    "> "
                } else {
                    "  "
                };
                let style = if idx == self.feature_toggle_selected {
                    selected_style
                } else {
                    normal_style
                };
                Line::from(Span::styled(format!("{marker}{row}"), style))
            })
            .collect();

        let mut content = vec![
            Line::from(Span::styled(
                "Toggle optional features before continuing.",
                Style::default().fg(Color::LightYellow),
            )),
            Line::from(""),
        ];
        content.extend(lines);
        content.push(Line::from(""));
        content.push(Line::from(
            "Use Up/Down to navigate. Enter or Space toggles Git/SSH. Select Continue and Enter to proceed.",
        ));

        let p = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightBlue))
                    .title("Programs"),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    }

    fn draw_existing_confirm(&self, f: &mut Frame<'_>, area: Rect) {
        let disk = &self.disks[self.selected_disk];
        let overwrite_style = if self.existing_overwrite_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let keep_style = if self.existing_overwrite_selected {
            Style::default().fg(Color::Gray)
        } else {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightRed)
                .add_modifier(Modifier::BOLD)
        };
        let mut lines = vec![
            Line::from(vec![Span::styled(
                "Existing install markers detected on selected disk.",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )]),
            Line::from(""),
            Line::from(format!("Disk: {} ({}, {})", disk.path, disk.size, disk.model)),
            Line::from(""),
            Line::from("Use Left/Right or Up/Down to choose, Enter to confirm."),
            Line::from(""),
            Line::from(vec![
                Span::styled("[ Overwrite ]", overwrite_style),
                Span::raw("   "),
                Span::styled("[ Keep / Exit ]", keep_style),
            ]),
        ];

        let p = Paragraph::new(lines.split_off(0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title("Existing Configuration"),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    }

    fn draw_final_confirm(&self, f: &mut Frame<'_>, area: Rect) {
        let disk = &self.disks[self.selected_disk];
        let proceed_style = if self.final_proceed_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let cancel_style = if self.final_proceed_selected {
            Style::default().fg(Color::Gray)
        } else {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightRed)
                .add_modifier(Modifier::BOLD)
        };
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    "WARNING: ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw("This will erase the selected disk."),
            ]),
            Line::from(""),
            Line::from(format!("Disk: {} ({}, {})", disk.path, disk.size, disk.model)),
            Line::from(format!("Boot: {}", self.boot_size)),
            Line::from(format!("Swap: {}", self.swap_size)),
            Line::from(format!(
                "Encryption: {}",
                if self.enable_encryption {
                    "enabled"
                } else {
                    "disabled"
                }
            )),
            Line::from(format!(
                "Flakes: {}",
                if self.enable_flakes {
                    "enabled"
                } else {
                    "disabled"
                }
            )),
            Line::from(format!("Hostname: {}", self.hostname)),
            Line::from(format!("Timezone: {}", self.timezone)),
            Line::from(format!("Keyboard: {}", self.keyboard_layout)),
            Line::from(format!("User: {}", self.username)),
            Line::from(format!(
                "Sudo: {}",
                if self.enable_sudo { "enabled" } else { "disabled" }
            )),
            Line::from(format!(
                "NetworkManager: {}",
                if self.install_networkmanager {
                    "enabled"
                } else {
                    "disabled"
                }
            )),
            Line::from(format!(
                "Git: {}",
                if self.enable_git { "enabled" } else { "disabled" }
            )),
            Line::from(format!(
                "SSH: {}",
                if self.enable_ssh { "enabled" } else { "disabled" }
            )),
            Line::from(format!(
                "ZFS profile: {}",
                if self.zfs_use_recommended {
                    "recommended"
                } else {
                    "advanced"
                }
            )),
            Line::from(format!("ZFS ashift: {}", self.zfs_ashift)),
            Line::from(format!("ZFS redundancy: {}", self.zfs_redundancy)),
            Line::from(format!("ZFS compression: {}", self.zfs_compression)),
            Line::from(format!("ZFS cache: {}", self.zfs_primarycache)),
            Line::from(format!(
                "ZFS autotrim: {}",
                if self.zfs_autotrim { "on" } else { "off" }
            )),
        ];

        if !self.warnings.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Warnings:",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )));
            for warn in &self.warnings {
                lines.push(Line::from(format!("- {warn}")));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from("Use Left/Right or Up/Down to choose, Enter to confirm."));
        lines.push(Line::from("F10 is a quick shortcut to proceed."));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("[ Proceed ]", proceed_style),
            Span::raw("   "),
            Span::styled("[ Cancel ]", cancel_style),
        ]));

        let popup = centered_rect(80, 70, area);
        f.render_widget(Clear, popup);
        let p = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::LightRed))
                    .title("Final Confirmation"),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(p, popup);
    }
}

pub struct TuiSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TuiSession {
    pub fn start() -> Result<Self> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, TermClear(ClearType::All), MoveTo(0, 0))?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(Self { terminal })
    }

    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    pub fn stop(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for TuiSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
