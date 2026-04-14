use std::collections::HashSet;
use std::fs;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear as TermClear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::execute;
use rand::Rng;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use regex::Regex;
use serde::de::Deserializer;
use serde::Deserialize;
use tempfile::NamedTempFile;

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

fn main() {
    if let Err(err) = run() {
        eprintln!("\nERROR: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    preflight_checks()?;
    check_required_commands()?;

    let mut app = App::new(load_disks()?);
    if app.disks.is_empty() {
        bail!("No writable disks were found via lsblk.");
    }

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
            println!(
                "Encryption:  {}",
                if cfg.enable_encryption { "enabled" } else { "disabled" }
            );
            println!(
                "Flakes:      {}",
                if cfg.enable_flakes { "enabled" } else { "disabled" }
            );
            println!();
            run_installer(&cfg)
        }
    }
}

fn preflight_checks() -> Result<()> {
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

fn check_required_commands() -> Result<()> {
    let mut missing = Vec::new();
    for cmd in REQUIRED_COMMANDS {
        if which::which(cmd).is_err() {
            missing.push(*cmd);
        }
    }
    if !missing.is_empty() {
        bail!(
            "Missing required tools in PATH: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct Disk {
    path: String,
    size: String,
    model: String,
}

#[derive(Clone)]
struct InstallConfig {
    disk: Disk,
    boot_size: String,
    swap_size: String,
    passphrase: String,
    enable_encryption: bool,
    enable_flakes: bool,
}

enum FinalAction {
    Exit,
    Install(InstallConfig),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UiScreen {
    DiskSelect,
    BootSize,
    SwapSize,
    EncryptionChoice,
    Passphrase,
    PassphraseConfirm,
    FlakesChoice,
    ExistingConfirm,
    FinalConfirm,
}

struct App {
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
    existing_found: bool,
    choice_yes_selected: bool,
    existing_overwrite_selected: bool,
    final_proceed_selected: bool,
    warnings: Vec<String>,
    status: String,
}

impl App {
    fn new(disks: Vec<Disk>) -> Self {
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
            existing_found: false,
            choice_yes_selected: true,
            existing_overwrite_selected: true,
            final_proceed_selected: true,
            warnings: Vec::new(),
            status: "Select installation disk with arrow keys, Enter to continue.".to_string(),
        }
    }

    fn run_tui(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<FinalAction> {
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
                let cfg = InstallConfig {
                    disk: self.disks[self.selected_disk].clone(),
                    boot_size: self.boot_size.clone(),
                    swap_size: self.swap_size.clone(),
                    passphrase: self.passphrase.clone(),
                    enable_encryption: self.enable_encryption,
                    enable_flakes: self.enable_flakes,
                };
                return Ok(Some(FinalAction::Install(cfg)));
            }
            KeyCode::F(10) => {
                let cfg = InstallConfig {
                    disk: self.disks[self.selected_disk].clone(),
                    boot_size: self.boot_size.clone(),
                    swap_size: self.swap_size.clone(),
                    passphrase: self.passphrase.clone(),
                    enable_encryption: self.enable_encryption,
                    enable_flakes: self.enable_flakes,
                };
                return Ok(Some(FinalAction::Install(cfg)));
            }
            _ => {}
        }
        Ok(None)
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
                Span::styled("> Value: ", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
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
            Style::default().fg(Color::Black).bg(Color::LightGreen).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let no_style = if yes_selected {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::Black).bg(Color::LightRed).add_modifier(Modifier::BOLD)
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

    fn draw_existing_confirm(&self, f: &mut Frame<'_>, area: Rect) {
        let disk = &self.disks[self.selected_disk];
        let overwrite_style = if self.existing_overwrite_selected {
            Style::default().fg(Color::Black).bg(Color::LightGreen).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let keep_style = if self.existing_overwrite_selected {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::Black).bg(Color::LightRed).add_modifier(Modifier::BOLD)
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
            Style::default().fg(Color::Black).bg(Color::LightGreen).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let cancel_style = if self.final_proceed_selected {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::Black).bg(Color::LightRed).add_modifier(Modifier::BOLD)
        };
        let mut lines = vec![
            Line::from(vec![
                Span::styled("WARNING: ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw("This will erase the selected disk."),
            ]),
            Line::from(""),
            Line::from(format!("Disk: {} ({}, {})", disk.path, disk.size, disk.model)),
            Line::from(format!("Boot: {}", self.boot_size)),
            Line::from(format!("Swap: {}", self.swap_size)),
            Line::from(format!(
                "Encryption: {}",
                if self.enable_encryption { "enabled" } else { "disabled" }
            )),
            Line::from(format!(
                "Flakes: {}",
                if self.enable_flakes { "enabled" } else { "disabled" }
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

struct TuiSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TuiSession {
    fn start() -> Result<Self> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, TermClear(ClearType::All), MoveTo(0, 0))?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(Self { terminal })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    fn stop(&mut self) -> Result<()> {
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

fn load_disks() -> Result<Vec<Disk>> {
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

fn validate_size_input(value: &str, label: &str) -> Result<()> {
    let size_re = Regex::new(r"^[1-9][0-9]*(K|M|G|T)$").unwrap();
    if !size_re.is_match(value) {
        bail!(
            "{} size must match pattern like 512M, 1G, 16G.",
            label
        );
    }
    Ok(())
}

fn has_existing_configuration(disk: &str) -> Result<bool> {
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

fn zpool_uses_disk(zpool_status: &str, disk: &str) -> bool {
    let disk_nvme = format!("{disk}p");
    for line in zpool_status.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(disk) || trimmed.starts_with(&disk_nvme) {
            return true;
        }
    }
    false
}

fn collect_disk_warnings(disk: &str) -> Result<Vec<String>> {
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

fn run_installer(cfg: &InstallConfig) -> Result<()> {
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

    if cfg.enable_encryption {
        println!("-> Creating encrypted zpool and datasets");
        let mut keyfile = NamedTempFile::new_in("/tmp")?;
        use std::io::Write;
        keyfile.write_all(cfg.passphrase.as_bytes())?;
        keyfile.write_all(b"\n")?;
        let keypath = keyfile.path().to_string_lossy().to_string();

        run_command(
            "zpool",
            &[
                "create",
                "-f",
                "-o",
                "ashift=12",
                "-o",
                "autotrim=on",
                "-O",
                "compression=lz4",
                "-O",
                "acltype=posixacl",
                "-O",
                "atime=off",
                "-O",
                "xattr=sa",
                "-O",
                "normalization=formD",
                "-O",
                "mountpoint=none",
                "-O",
                "encryption=aes-256-gcm",
                "-O",
                "keyformat=passphrase",
                "-O",
                &format!("keylocation=file://{keypath}"),
                "zroot",
                &part_zfs,
            ],
        )?;

        run_command("zfs", &["set", "keylocation=prompt", "zroot"])?;
    } else {
        println!("-> Creating unencrypted zpool and datasets");
        run_command(
            "zpool",
            &[
                "create",
                "-f",
                "-o",
                "ashift=12",
                "-o",
                "autotrim=on",
                "-O",
                "compression=lz4",
                "-O",
                "acltype=posixacl",
                "-O",
                "atime=off",
                "-O",
                "xattr=sa",
                "-O",
                "normalization=formD",
                "-O",
                "mountpoint=none",
                "zroot",
                &part_zfs,
            ],
        )?;
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

    let flakes_snippet = if cfg.enable_flakes {
        "  nix.settings.experimental-features = [ \"nix-command\" \"flakes\" ];\n\n"
    } else {
        ""
    };
    let zfs_module = format!(
        "{{ config, pkgs, ... }}:\n\n{{\n  boot.loader.systemd-boot.enable = true;\n  boot.loader.efi.canTouchEfiVariables = true;\n  boot.loader.grub.enable = pkgs.lib.mkForce false;\n\n  networking.hostId = \"{host_id}\";\n  boot.supportedFilesystems = [ \"zfs\" ];\n  boot.zfs.devNodes = \"/dev/disk/by-partlabel\";\n\n{flakes_snippet}  swapDevices = pkgs.lib.mkForce [ {{\n    device = \"/dev/disk/by-partuuid/{swap_partuuid}\";\n    randomEncryption.enable = true;\n  }} ];\n}}\n"
    );
    fs::write("/mnt/etc/nixos/zfs.nix", zfs_module)?;

    let config_path = PathBuf::from("/mnt/etc/nixos/configuration.nix");
    let cfg_text = fs::read_to_string(&config_path)
        .context("Unable to read generated /mnt/etc/nixos/configuration.nix")?;

    let marker = "./hardware-configuration.nix";
    if !cfg_text.contains(marker) {
        bail!(
            "Could not find {marker} import in configuration.nix; please add ./zfs.nix manually."
        );
    }

    let replaced = cfg_text.replacen(marker, "./hardware-configuration.nix ./zfs.nix", 1);
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

fn run_command(binary: &str, args: &[&str]) -> Result<()> {
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

fn run_command_capture(binary: &str, args: &[&str]) -> Result<String> {
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

fn run_command_allow_fail(binary: &str, args: &[&str]) -> Result<()> {
    let _ = Command::new(binary).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    Ok(())
}
