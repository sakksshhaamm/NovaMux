//! Root-confined, read-only file explorer state.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_ENTRIES: usize = 4096;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);

/// Kind of a directory entry. Symlinks are never followed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

/// Display metadata for one entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub bytes: Option<u64>,
}

/// A read-only browser confined to the canonical directory where it was opened.
#[derive(Debug)]
pub struct FileExplorer {
    root: PathBuf,
    current: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    offset: usize,
    error: Option<String>,
    last_click: Option<(usize, Instant)>,
}

impl FileExplorer {
    /// Opens a browser rooted at the canonical form of `root`.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be resolved, inspected, or is not
    /// a directory.
    pub fn open(root: &Path) -> io::Result<Self> {
        let root = fs::canonicalize(root)?;
        if !fs::metadata(&root)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file explorer root is not a directory",
            ));
        }
        let mut explorer = Self {
            current: root.clone(),
            root,
            entries: Vec::new(),
            selected: 0,
            offset: 0,
            error: None,
            last_click: None,
        };
        explorer.refresh();
        Ok(explorer)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn current(&self) -> &Path {
        &self.current
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    #[must_use]
    pub fn selected(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    pub fn refresh(&mut self) {
        match read_entries(&self.current) {
            Ok(entries) => {
                self.entries = entries;
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
                self.offset = self.offset.min(self.selected);
                self.error = None;
            }
            Err(error) => {
                self.entries.clear();
                self.selected = 0;
                self.offset = 0;
                self.error = Some(format!("cannot read directory: {error}"));
            }
        }
    }

    pub fn move_selection(&mut self, delta: isize, visible_rows: usize) {
        if self.entries.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.entries.len() - 1);
        self.ensure_visible(visible_rows);
    }

    pub fn page(&mut self, delta: isize, visible_rows: usize) {
        self.move_selection(
            delta.saturating_mul(isize::try_from(visible_rows.max(1)).unwrap_or(isize::MAX)),
            visible_rows,
        );
    }

    pub fn select_visible_row(&mut self, row: usize, visible_rows: usize, now: Instant) -> bool {
        let index = self.offset.saturating_add(row);
        if index >= self.entries.len() {
            return false;
        }
        let double = self.last_click.is_some_and(|(prior, at)| {
            prior == index && now.duration_since(at) <= DOUBLE_CLICK_INTERVAL
        });
        self.selected = index;
        self.ensure_visible(visible_rows);
        self.last_click = Some((index, now));
        double
    }

    pub fn enter_selected(&mut self) {
        let Some(entry) = self.selected().cloned() else {
            return;
        };
        if entry.kind != EntryKind::Directory {
            self.error = Some(if entry.kind == EntryKind::Symlink {
                "symlinks are labeled but not followed".to_owned()
            } else {
                "selected item is not a directory".to_owned()
            });
            return;
        }
        match fs::canonicalize(&entry.path) {
            Ok(path) if path.starts_with(&self.root) => {
                self.current = path;
                self.selected = 0;
                self.offset = 0;
                self.refresh();
            }
            Ok(_) => {
                self.error = Some("navigation outside the explorer root was blocked".to_owned());
            }
            Err(error) => self.error = Some(format!("cannot open directory: {error}")),
        }
    }

    pub fn parent(&mut self) {
        if self.current == self.root {
            self.error = Some("already at explorer root".to_owned());
            return;
        }
        let Some(parent) = self.current.parent() else {
            return;
        };
        match fs::canonicalize(parent) {
            Ok(path) if path.starts_with(&self.root) => {
                self.current = path;
                self.selected = 0;
                self.offset = 0;
                self.refresh();
            }
            _ => self.error = Some("navigation outside the explorer root was blocked".to_owned()),
        }
    }

    fn ensure_visible(&mut self, rows: usize) {
        let rows = rows.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset.saturating_add(rows) {
            self.offset = self.selected + 1 - rows;
        }
    }
}

fn read_entries(directory: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(directory)? {
        if entries.len() == MAX_ENTRIES {
            return Err(io::Error::other(format!(
                "directory exceeds the {MAX_ENTRIES}-entry safety limit"
            )));
        }
        let item = item?;
        let metadata = fs::symlink_metadata(item.path())?;
        let kind = if metadata.file_type().is_symlink() {
            EntryKind::Symlink
        } else if metadata.is_dir() {
            EntryKind::Directory
        } else if metadata.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        entries.push(Entry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: item.path(),
            kind,
            bytes: (kind == EntryKind::File).then_some(metadata.len()),
        });
    }
    entries.sort_by(|left, right| {
        entry_rank(left.kind)
            .cmp(&entry_rank(right.kind))
            .then_with(|| casefold(&left.name).cmp(&casefold(&right.name)))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(entries)
}

const fn entry_rank(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Directory => 0,
        EntryKind::File => 1,
        EntryKind::Symlink => 2,
        EntryKind::Other => 3,
    }
}

fn casefold(value: &str) -> String {
    value.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};

    fn sandbox(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("novamux-explorer-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn sorts_directories_before_files_and_reports_metadata() {
        let root = sandbox("sort");
        File::create(root.join("z.txt")).unwrap();
        fs::create_dir(root.join("A-dir")).unwrap();
        let explorer = FileExplorer::open(&root).unwrap();
        assert_eq!(explorer.entries()[0].kind, EntryKind::Directory);
        assert_eq!(explorer.entries()[1].bytes, Some(0));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parent_cannot_escape_canonical_root() {
        let root = sandbox("parent");
        let mut explorer = FileExplorer::open(&root).unwrap();
        explorer.parent();
        assert_eq!(explorer.current(), explorer.root());
        assert!(explorer.error().unwrap().contains("root"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_labeled_and_never_followed() {
        use std::os::unix::fs::symlink;
        let root = sandbox("symlink");
        symlink(root.parent().unwrap(), root.join("outside")).unwrap();
        let mut explorer = FileExplorer::open(&root).unwrap();
        assert_eq!(explorer.selected().unwrap().kind, EntryKind::Symlink);
        explorer.enter_selected();
        assert_eq!(explorer.current(), explorer.root());
        assert!(explorer.error().unwrap().contains("not followed"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scrolling_and_hit_testing_remain_bounded() {
        let root = sandbox("scroll");
        for index in 0..20 {
            File::create(root.join(format!("{index:02}.txt"))).unwrap();
        }
        let mut explorer = FileExplorer::open(&root).unwrap();
        explorer.move_selection(12, 5);
        assert_eq!(explorer.offset(), 8);
        let now = Instant::now();
        assert!(!explorer.select_visible_row(2, 5, now));
        assert!(explorer.select_visible_row(2, 5, now + Duration::from_millis(10)));
        explorer.move_selection(-100, 5);
        assert_eq!(explorer.offset(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unicode_names_are_preserved_without_affecting_sort_order() {
        let root = sandbox("unicode");
        File::create(root.join("🌸-notes.txt")).unwrap();
        File::create(root.join("éclair.txt")).unwrap();
        let explorer = FileExplorer::open(&root).unwrap();
        assert!(
            explorer
                .entries()
                .iter()
                .any(|entry| entry.name == "🌸-notes.txt")
        );
        assert!(
            explorer
                .entries()
                .iter()
                .any(|entry| entry.name == "éclair.txt")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_directory_removed_during_browsing_surfaces_an_error() {
        let root = sandbox("race");
        fs::create_dir(root.join("child")).unwrap();
        let mut explorer = FileExplorer::open(&root).unwrap();
        explorer.enter_selected();
        fs::remove_dir(explorer.current()).unwrap();
        explorer.refresh();
        assert!(explorer.entries().is_empty());
        assert!(explorer.error().unwrap().contains("cannot read"));
        fs::remove_dir_all(root).unwrap();
    }
}
