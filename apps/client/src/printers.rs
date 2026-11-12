//! The printers of this device (M12.4, M12.5): each with its connection,
//! its paper, what it prints (receipts, invoices, labels), copies, cutting
//! and the cash drawer. Kept on the device (printers are where the device
//! is), as JSON in its settings storage. Sending is plain IO, off the
//! interface's thread.

use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How bytes reach the printer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Link {
    /// ESC/POS or TSPL over the network: `host` or `host:port` (9100).
    Network { addr: String },
    /// Raw bytes written to a device or a shared printer: `/dev/usb/lp0`
    /// on Linux, `\\localhost\XP58` (a shared USB printer) on Windows.
    Device { path: String },
    /// A printer of the system (CUPS on Linux and macOS, the Windows
    /// spooler): sheets go to it as PDF.
    System { queue: String },
}

impl Default for Link {
    fn default() -> Self {
        Link::Network {
            addr: String::new(),
        }
    }
}

/// The paper loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sheet {
    /// A roll `paper_mm` wide printing `print_mm` across.
    Roll {
        paper_mm: u16,
        print_mm: u16,
    },
    /// Labels `w_mm` × `h_mm` with `gap_mm` between them.
    Label {
        w_mm: u16,
        h_mm: u16,
        gap_mm: u16,
    },
    A4,
    A5,
}

impl Default for Sheet {
    fn default() -> Self {
        Sheet::Roll {
            paper_mm: 80,
            print_mm: 72,
        }
    }
}

impl Sheet {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn paper(self) -> nb_docs::Paper {
        match self {
            Sheet::Roll { print_mm, .. } => nb_docs::Paper::roll_mm(print_mm),
            Sheet::Label { w_mm, h_mm, .. } => nb_docs::Paper::Label { w_mm, h_mm },
            Sheet::A4 => nb_docs::Paper::A4,
            Sheet::A5 => nb_docs::Paper::A5,
        }
    }

    pub fn is_sheet(self) -> bool {
        matches!(self, Sheet::A4 | Sheet::A5)
    }
}

/// What a printer is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Receipt,
    Invoice,
    Label,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Receipt, Role::Invoice, Role::Label];

    pub fn key(self) -> &'static str {
        match self {
            Role::Receipt => "printers.role.receipt",
            Role::Invoice => "printers.role.invoice",
            Role::Label => "printers.role.label",
        }
    }
}

/// One printer of this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Printer {
    pub id: u32,
    pub name: String,
    pub link: Link,
    pub sheet: Sheet,
    pub roles: Vec<Role>,
    pub copies: u8,
    /// Cut after each receipt (thermal rolls).
    #[serde(default = "yes")]
    pub cut: bool,
    /// Open the cash drawer wired to it after a cash sale.
    #[serde(default)]
    pub drawer: bool,
    /// Labels: TSPL (label printers); else ESC/POS raster.
    #[serde(default = "yes")]
    pub tspl: bool,
}

fn yes() -> bool {
    true
}

impl Default for Printer {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            link: Link::default(),
            sheet: Sheet::default(),
            roles: vec![Role::Receipt],
            copies: 1,
            cut: true,
            drawer: false,
            tspl: true,
        }
    }
}

impl Printer {
    /// Thermal (raw bytes): a roll or labels on a network or device link.
    pub fn is_thermal(&self) -> bool {
        !self.sheet.is_sheet()
    }

    /// Whether everything needed to print is filled in.
    pub fn ready(&self) -> bool {
        match &self.link {
            Link::Network { addr } => !addr.trim().is_empty(),
            Link::Device { path } => !path.trim().is_empty(),
            Link::System { queue } => !queue.trim().is_empty(),
        }
    }
}

/// The printers kept, from their JSON (an unreadable list is empty).
pub fn parse(json: &str) -> Vec<Printer> {
    serde_json::from_str(json).unwrap_or_default()
}

pub fn to_json(list: &[Printer]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".into())
}

/// The first printer for `role`, if any is ready.
pub fn for_role(list: &[Printer], role: Role) -> Option<Printer> {
    list.iter()
        .find(|p| p.roles.contains(&role) && p.ready())
        .cloned()
}

/// A printer from the settings of the first version (one network
/// address, one width), so nothing is lost on update.
pub fn from_old(
    addr: &str,
    width_index: usize,
    copies_index: usize,
    drawer: bool,
) -> Option<Printer> {
    let addr = addr.trim();
    if addr.is_empty() {
        return None;
    }
    let (paper_mm, print_mm) = if width_index == 0 { (58, 48) } else { (80, 72) };
    Some(Printer {
        id: 1,
        name: "ESC/POS".into(),
        link: Link::Network { addr: addr.into() },
        sheet: Sheet::Roll { paper_mm, print_mm },
        roles: vec![Role::Receipt],
        copies: u8::try_from(copies_index + 1).unwrap_or(1),
        drawer,
        ..Printer::default()
    })
}

// ------------------------------------------------------------- transport

#[cfg(not(target_arch = "wasm32"))]
pub mod send {
    use super::*;

    /// Sends `bytes` to the printer at `addr` (`host` or `host:port`).
    pub fn network(addr: &str, bytes: &[u8]) -> std::io::Result<()> {
        let addr = addr.trim();
        let addr = if addr.contains(':') {
            addr.to_owned()
        } else {
            format!("{addr}:9100")
        };
        let to = addr
            .to_socket_addrs()?
            .next()
            .ok_or(std::io::ErrorKind::NotFound)?;
        let mut s = TcpStream::connect_timeout(&to, Duration::from_secs(3))?;
        s.set_write_timeout(Some(Duration::from_secs(15)))?;
        s.write_all(bytes)?;
        s.flush()
    }

    /// Writes `bytes` to a device or a shared printer.
    pub fn device(path: &str, bytes: &[u8]) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(false)
            .open(path.trim())?;
        f.write_all(bytes)?;
        f.flush()
    }

    /// The command that prints a PDF file on a system printer.
    pub fn system_command(
        queue: &str,
        file: &std::path::Path,
        copies: u8,
    ) -> (String, Vec<String>) {
        let file = file.display().to_string();
        if cfg!(target_os = "windows") {
            // The spooler through the PDF reader's "print to" verb.
            let script = format!(
                "Start-Process -FilePath '{}' -Verb PrintTo -ArgumentList '\"{}\"' -WindowStyle Hidden",
                file.replace('\'', "''"),
                queue.replace('\'', "''").replace('"', "")
            );
            (
                "powershell".into(),
                vec!["-NoProfile".into(), "-Command".into(), script],
            )
        } else {
            (
                "lp".into(),
                vec![
                    "-d".into(),
                    queue.trim().into(),
                    "-n".into(),
                    copies.max(1).to_string(),
                    file,
                ],
            )
        }
    }

    /// Prints a PDF on a system printer.
    pub fn system(queue: &str, pdf: &[u8], copies: u8) -> Result<(), String> {
        let dir = std::env::temp_dir().join("nsh-business-print");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let file = dir.join(format!(
            "doc-{}.pdf",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis())
        ));
        std::fs::write(&file, pdf).map_err(|e| e.to_string())?;
        let (cmd, args) = system_command(queue, &file, copies);
        let runs = if cfg!(target_os = "windows") {
            copies.max(1)
        } else {
            1
        };
        for _ in 0..runs {
            let out = std::process::Command::new(&cmd)
                .args(&args)
                .output()
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).into_owned());
            }
        }
        Ok(())
    }

    /// The system's printers (names of queues), best effort.
    pub fn system_printers() -> Vec<String> {
        let out = if cfg!(target_os = "windows") {
            std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "Get-Printer | Select-Object -ExpandProperty Name",
                ])
                .output()
        } else {
            std::process::Command::new("lpstat").arg("-e").output()
        };
        match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printers_keep_as_json_and_old_settings_carry_over() {
        let list = vec![
            Printer {
                id: 1,
                name: "Caisse".into(),
                ..Printer::default()
            },
            Printer {
                id: 2,
                name: "Étiquettes".into(),
                link: Link::Device {
                    path: "/dev/usb/lp1".into(),
                },
                sheet: Sheet::Label {
                    w_mm: 40,
                    h_mm: 30,
                    gap_mm: 2,
                },
                roles: vec![Role::Label],
                ..Printer::default()
            },
        ];
        assert_eq!(parse(&to_json(&list)), list);
        assert!(parse("not json").is_empty());
        // Not ready without an address.
        assert_eq!(for_role(&list, Role::Receipt), None);
        assert_eq!(for_role(&list, Role::Label).map(|p| p.id), Some(2));
        let old = from_old("192.168.1.50", 0, 1, true);
        assert_eq!(
            old.as_ref().map(|p| (p.sheet, p.copies, p.drawer)),
            Some((
                Sheet::Roll {
                    paper_mm: 58,
                    print_mm: 48
                },
                2,
                true
            ))
        );
        assert_eq!(from_old("  ", 1, 0, false), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn bytes_reach_a_device_and_the_system_command_is_right() {
        let dir = std::env::temp_dir().join(format!("nb-printers-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("lp0");
        // A device that is not there is an error (never created).
        assert!(send::device(&path.display().to_string(), b"x").is_err());
        let _ = std::fs::write(&path, b"");
        assert!(send::device(&path.display().to_string(), b"\x1b@hello").is_ok());
        assert_eq!(std::fs::read(&path).unwrap_or_default(), b"\x1b@hello");
        let (cmd, args) =
            send::system_command("HP_LaserJet", std::path::Path::new("/tmp/a.pdf"), 2);
        if !cfg!(target_os = "windows") {
            assert_eq!(cmd, "lp");
            assert_eq!(args, ["-d", "HP_LaserJet", "-n", "2", "/tmp/a.pdf"]);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
