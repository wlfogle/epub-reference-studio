use crate::book;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const FLATPAK_ID: &str = "com.sigil_ebook.Sigil";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditingCopy {
    pub path: PathBuf,
    pub source: PathBuf,
    pub original_sha256: String,
    pub created: u64,
    #[serde(default)]
    pub process: Option<EditorProcess>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorProcess {
    pub pid: u32,
    pub started: Option<String>,
}

fn process_start(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    if fields.first().copied() == Some("Z") {
        return None;
    }
    fields.get(19).map(|s| s.to_string())
}

pub fn running(process: &EditorProcess) -> bool {
    process.started.is_some() && process_start(process.pid) == process.started
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Book,
    Finding { index: usize },
    Change { id: String },
    Export { path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub path: PathBuf,
    pub member: Option<String>,
    pub line: Option<usize>,
    pub search_text: String,
    pub context: String,
    pub notice: String,
    pub editor: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Editor {
    pub label: String,
    pub executable: PathBuf,
    pub flatpak: bool,
}

fn executable(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        let p = directory.join(name);
        (p.is_absolute() && p.is_file() && p.metadata().ok()?.permissions().mode() & 0o111 != 0)
            .then_some(p)
    })
}

pub fn detect() -> Option<Editor> {
    if let Some(path) = executable("sigil") {
        return Some(Editor {
            label: "Sigil (native)".into(),
            executable: path,
            flatpak: false,
        });
    }
    let flatpak = executable("flatpak")?;
    Command::new(&flatpak)
        .args(["info", FLATPAK_ID])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success()
        .then_some(Editor {
            label: "Sigil (Flatpak)".into(),
            executable: flatpak,
            flatpak: true,
        })
}

pub fn arguments(editor: &Editor, path: &Path) -> Vec<std::ffi::OsString> {
    if editor.flatpak {
        ["run", "--file-forwarding", FLATPAK_ID, "@@"]
            .into_iter()
            .map(Into::into)
            .chain([path.as_os_str().to_owned(), "@@".into()])
            .collect()
    } else {
        vec![path.as_os_str().to_owned()]
    }
}

pub fn launch(editor: &Editor, path: &Path, log: File) -> Result<EditorProcess> {
    let mut child = Command::new(&editor.executable)
        .args(arguments(editor, path))
        .env_remove("LD_PRELOAD")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .context("Could not start Sigil")?;
    std::thread::sleep(Duration::from_millis(350));
    let process = EditorProcess {
        pid: child.id(),
        started: process_start(child.id()),
    };
    if let Some(status) = child.try_wait()? {
        ensure!(
            status.success(),
            "Sigil launch failed ({status}); see the private sigil-launch.log"
        );
    } else {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(process)
}

/// Locate a visible-text offset without modifying XHTML, reading order or IDs.
pub fn locate(book: &book::Book, member: &str, start: usize, end: usize) -> Handoff {
    let mut handoff = Handoff {
        path: PathBuf::new(), member: Some(member.into()), line: None,
        search_text: String::new(), context: book::context(&book.text, start, end),
        notice: "Sigil accepts the EPUB filename, not a passage-jump argument. In Book Browser open the listed XHTML file. Use the line hint or copy the search text into Find & Replace in Normal mode. Text split by formatting may require the line hint.".into(),
        editor: String::new(),
    };
    if let Some(s) = book
        .segments
        .iter()
        .find(|s| s.member == member && s.text.start <= start && start < s.text.end)
    {
        if let Some(raw) = book.documents.get(member) {
            handoff.line = Some(raw[..s.raw.start].bytes().filter(|&c| c == b'\n').count() + 1);
            // A raw-node excerpt remains searchable when visible words span tags.
            let fragment = &raw[s.raw.clone()];
            let relative = start - s.text.start;
            let direct = fragment == s.decoded;
            let offset = if direct {
                relative.min(fragment.len())
            } else {
                0
            };
            let mut a = offset.saturating_sub(25);
            let mut b = (offset + 90).min(fragment.len());
            while !fragment.is_char_boundary(a) {
                a += 1;
            }
            while !fragment.is_char_boundary(b) {
                b -= 1;
            }
            // Keep a single line for Sigil's Normal search field.
            let snippet = fragment[a..b]
                .split(['\n', '\r'])
                .find(|s| !s.trim().is_empty())
                .unwrap_or("")
                .trim();
            handoff.search_text = snippet.to_owned();
            if direct {
                handoff.line = Some(
                    raw[..s.raw.start + offset]
                        .bytes()
                        .filter(|&c| c == b'\n')
                        .count()
                        + 1,
                );
            }
        }
    }
    handoff
}

pub fn existing_path(root: &Path, path: &Path) -> Result<PathBuf> {
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Editing copy cannot be a symlink"
    );
    let path = path.canonicalize()?;
    let root = root.canonicalize()?;
    ensure!(
        path.starts_with(root) && path.is_file(),
        "Editing copy escaped the private workspace"
    );
    Ok(path)
}

pub fn copy(source: &Path, destination: &Path, hash: &str) -> Result<()> {
    ensure!(
        book::file_hash(source)? == hash,
        "Source changed; analyze again"
    );
    let parent = destination.parent().context("Editing copy has no parent")?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut File::open(source)?, temp.as_file_mut())?;
    temp.flush()?;
    temp.as_file_mut().sync_all()?;
    ensure!(
        book::file_hash(temp.path())? == hash && book::file_hash(source)? == hash,
        "Book changed while preparing editing copy"
    );
    temp.persist_noclobber(destination)
        .map_err(|e| anyhow::anyhow!("Cannot publish editing copy: {e}"))?;
    Ok(())
}

/// Optional explicit clipboard action; never reads the existing clipboard.
pub fn copy_search(text: &str) -> Result<()> {
    ensure!(
        !text.is_empty() && text.len() <= 1024 && !text.contains('\0'),
        "No valid search text"
    );
    let mut command =
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && executable("wl-copy").is_some() {
            Command::new(executable("wl-copy").context("wl-copy is unavailable")?)
        } else {
            let mut command = Command::new(executable("xsel").context(
                "Clipboard helper unavailable: select the displayed search text and press Ctrl+C",
            )?);
            command.args(["--clipboard", "--input"]);
            command
        };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .context("Clipboard stdin unavailable")?
        .write_all(text.as_bytes())?;
    ensure!(
        child.wait()?.success(),
        "Clipboard write failed; select the text and press Ctrl+C"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatpak_forwarding_and_native_filename_are_single_arguments() {
        let path = Path::new("/tmp/book ; echo $HOME.epub");
        let mut editor = Editor {
            label: "fixture".into(),
            executable: "/usr/bin/flatpak".into(),
            flatpak: true,
        };
        let args = arguments(&editor, path);
        assert_eq!(
            args,
            vec![
                "run",
                "--file-forwarding",
                FLATPAK_ID,
                "@@",
                "/tmp/book ; echo $HOME.epub",
                "@@",
            ]
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
        );
        editor.flatpak = false;
        assert_eq!(arguments(&editor, path), vec![path.as_os_str()]);
    }

    #[test]
    fn process_identity_and_clipboard_bounds() {
        let process = EditorProcess {
            pid: std::process::id(),
            started: process_start(std::process::id()),
        };
        assert!(running(&process));
        assert!(!running(&EditorProcess {
            started: Some("wrong-start".into()),
            ..process
        }));
        assert!(copy_search("").is_err());
        assert!(copy_search(&"x".repeat(1025)).is_err());
    }
}
