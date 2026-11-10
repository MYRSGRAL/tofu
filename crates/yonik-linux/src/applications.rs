//! Ограниченное чтение каталога по запросу. Не запускает приложения и не атрибутирует потоки.
use rustix::fs::{Mode, OFlags, open, openat};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read},
    os::fd::AsRawFd,
    path::{Component, Path, PathBuf},
};
const DIR: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_dirs: usize,
    pub max_depth: usize,
    pub max_files: usize,
    pub max_bytes: usize,
    pub max_entries: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_dirs: 128,
            max_depth: 8,
            max_files: 8192,
            max_bytes: 65536,
            max_entries: 4096,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsupported {
    MissingExec,
    UnresolvedPath,
    ComplexExec,
    WrapperOrInterpreter,
    PackageFormat,
    UnverifiedExecutable,
}
#[derive(Debug)]
pub struct DesktopApplication {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub no_display: bool,
    pub executable: Result<PathBuf, Unsupported>,
}
#[derive(Debug, Clone)]
pub struct ProcessCandidate {
    pub pid: u32,
    pub start_time: u64,
    pub name: String,
    pub exe: Option<PathBuf>,
}
#[derive(Debug)]
pub struct Snapshot<T> {
    pub items: Vec<T>,
    pub incomplete: bool,
}
/// Корни applications в порядке XDG_DATA_HOME, затем XDG_DATA_DIRS.
/// Корни и locale задаёт вызывающая сторона; окружение и PATH не читаются.
pub struct Catalog {
    roots: Vec<PathBuf>,
    proc_root: PathBuf,
    locale: String,
    limits: Limits,
}
impl Catalog {
    pub fn new(
        roots: Vec<PathBuf>,
        proc_root: PathBuf,
        locale: String,
        limits: Limits,
    ) -> io::Result<Self> {
        if roots.len() > 128
            || roots
                .iter()
                .chain(std::iter::once(&proc_root))
                .any(|p| !safe_absolute(p))
            || locale.len() > 128
            || !locale
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.@-".contains(c))
            || limits.max_dirs == 0
            || limits.max_dirs > 4096
            || limits.max_depth > 32
            || limits.max_files == 0
            || limits.max_files > 100000
            || limits.max_entries == 0
            || limits.max_entries > 100000
            || limits.max_bytes == 0
            || limits.max_bytes > 1048576
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Невалидные корни, locale или лимиты",
            ));
        }
        Ok(Self {
            roots,
            proc_root,
            locale,
            limits,
        })
    }
    pub fn desktop_snapshot(&self) -> io::Result<Snapshot<DesktopApplication>> {
        self.desktop_snapshot_with(|dir, name| openat(dir, name, DIR, Mode::empty()))
    }
    fn desktop_snapshot_with(
        &self,
        mut open_child: impl FnMut(&File, &str) -> rustix::io::Result<std::os::fd::OwnedFd>,
    ) -> io::Result<Snapshot<DesktopApplication>> {
        let mut out = Snapshot {
            items: Vec::new(),
            incomplete: false,
        };
        let mut seen = BTreeSet::new();
        let mut files = 0;
        let mut dirs = 0;
        for root in &self.roots {
            if dirs >= self.limits.max_dirs {
                out.incomplete = true;
                break;
            }
            let dir = match directory(root) {
                Ok(d) => d,
                Err(_) => {
                    out.incomplete = true;
                    continue;
                }
            };
            dirs += 1;
            let mut stack = vec![(dir, String::new(), 0)];
            while let Some((dir, prefix, depth)) = stack.pop() {
                for name in names(
                    &dir,
                    self.limits.max_files.saturating_sub(files),
                    &mut out.incomplete,
                )? {
                    files += 1;
                    let relative = format!("{prefix}{name}");
                    // statat не открывает каталог: ожидающие в stack уже входят в dirs.
                    match rustix::fs::statat(
                        &dir,
                        name.as_str(),
                        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
                    ) {
                        Ok(stat)
                            if rustix::fs::FileType::from_raw_mode(stat.st_mode)
                                == rustix::fs::FileType::Directory =>
                        {
                            if dirs >= self.limits.max_dirs || depth >= self.limits.max_depth {
                                out.incomplete = true;
                                continue;
                            }
                            match open_child(&dir, name.as_str()) {
                                Ok(sub) => {
                                    dirs += 1;
                                    stack.push((
                                        File::from(sub),
                                        format!("{relative}/"),
                                        depth + 1,
                                    ));
                                }
                                Err(rustix::io::Errno::NOTDIR | rustix::io::Errno::NOENT) => {}
                                Err(_) => out.incomplete = true,
                            }
                            continue;
                        }
                        Err(rustix::io::Errno::NOENT) => {}
                        Err(_) => out.incomplete = true,
                        Ok(_) => {}
                    }
                    if !name.ends_with(".desktop") {
                        continue;
                    }
                    let id = relative.replace('/', "-");
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    let text = match read(&dir, &name, self.limits.max_bytes) {
                        Ok(t) => t,
                        Err(_) => {
                            out.incomplete = true;
                            continue;
                        }
                    };
                    if let Some(app) = parse(&text, id, &self.locale) {
                        if out.items.len() >= self.limits.max_entries {
                            out.incomplete = true;
                            return Ok(out);
                        }
                        out.items.push(app);
                    }
                }
            }
        }
        out.items.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }
    pub fn process_snapshot(&self) -> io::Result<Snapshot<ProcessCandidate>> {
        let root = directory(&self.proc_root)?;
        let mut out = Snapshot {
            items: Vec::new(),
            incomplete: false,
        };
        for name in names(&root, self.limits.max_files, &mut out.incomplete)? {
            let Ok(pid) = name.parse::<u32>() else {
                continue;
            };
            if pid == 0 {
                continue;
            }
            if out.items.len() >= self.limits.max_entries {
                out.incomplete = true;
                break;
            }
            match process(&root, &name, self.limits.max_bytes) {
                Ok(p) => {
                    out.incomplete |= p.exe.is_none();
                    out.items.push(p);
                }
                Err(_) => out.incomplete = true,
            }
        }
        out.items.sort_by_key(|p| p.pid);
        Ok(out)
    }
    pub fn process_is_current(&self, candidate: &ProcessCandidate) -> bool {
        directory(&self.proc_root)
            .and_then(|root| process(&root, &candidate.pid.to_string(), self.limits.max_bytes))
            .is_ok_and(|p| p.start_time == candidate.start_time && p.exe == candidate.exe)
    }
}
fn safe_absolute(p: &Path) -> bool {
    p.is_absolute()
        && p.components()
            .skip(1)
            .all(|c| matches!(c, Component::Normal(_)))
}
fn directory(p: &Path) -> io::Result<File> {
    let mut fd = File::from(open("/", DIR, Mode::empty())?);
    for c in p.components().skip(1) {
        let Component::Normal(n) = c else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        fd = File::from(openat(&fd, n, DIR, Mode::empty())?);
    }
    Ok(fd)
}
fn fd_path(dir: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()))
}
fn names(dir: &File, limit: usize, incomplete: &mut bool) -> io::Result<Vec<String>> {
    let mut result = Vec::new();
    for e in fs::read_dir(fd_path(dir))?.take(limit.saturating_add(1)) {
        if result.len() == limit {
            *incomplete = true;
            break;
        }
        let e = e?;
        if let Some(n) = e.file_name().to_str() {
            result.push(n.to_owned())
        } else {
            *incomplete = true
        }
    }
    result.sort();
    Ok(result)
}
fn read(dir: &File, name: &str, limit: usize) -> io::Result<String> {
    let file = File::from(openat(dir, name, READ, Mode::empty())?);
    if !file.metadata()?.is_file() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut data = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut data)?;
    if data.len() > limit {
        return Err(io::ErrorKind::InvalidData.into());
    }
    String::from_utf8(data).map_err(|_| io::ErrorKind::InvalidData.into())
}
fn unescape(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push(match chars.next()? {
                's' => ' ',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                _ => return None,
            });
        } else {
            out.push(c);
        }
    }
    Some(out)
}
fn parse(text: &str, id: String, locale: &str) -> Option<DesktopApplication> {
    let mut values = BTreeMap::new();
    let mut active = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            active = line == "[Desktop Entry]";
            continue;
        }
        if !active || line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (k, v) = line.split_once('=')?;
        if values.insert(k.trim(), v.trim()).is_some() {
            return None;
        }
    }
    if values.get("Hidden") == Some(&"true") || values.get("Type") != Some(&"Application") {
        return None;
    }
    let base = values.get("Name")?;
    let (langmod, modifier) = locale.split_once('@').unwrap_or((locale, ""));
    let langcountry = langmod.split('.').next()?;
    let lang = langcountry.split('_').next()?;
    let mut choices = Vec::new();
    if !modifier.is_empty() {
        choices.push(format!("{langcountry}@{modifier}"));
    }
    choices.push(langcountry.to_owned());
    if !modifier.is_empty() {
        choices.push(format!("{lang}@{modifier}"));
    }
    choices.push(lang.to_owned());
    let chosen = choices
        .iter()
        .find_map(|l| values.get(format!("Name[{l}]").as_str()))
        .unwrap_or(base);
    let name = unescape(chosen)?;
    if name.is_empty() {
        return None;
    }
    let executable = if values.get("DBusActivatable") == Some(&"true") {
        Err(Unsupported::PackageFormat)
    } else {
        selector(values.get("Exec").copied())
    };
    Some(DesktopApplication {
        id,
        name,
        icon: values.get("Icon").and_then(|v| unescape(v)),
        no_display: values.get("NoDisplay") == Some(&"true"),
        executable,
    })
}
fn selector(exec: Option<&str>) -> Result<PathBuf, Unsupported> {
    let exec = exec.ok_or(Unsupported::MissingExec)?;
    // Только однозначный поднабор Exec. Аргументы не исполняются и не сохраняются.
    if exec.chars().any(|c| "\"'\\`$;|&<>=\n\r".contains(c)) {
        return Err(Unsupported::ComplexExec);
    }
    if exec.split_whitespace().skip(1).any(|arg| {
        arg.contains('%') && !matches!(arg, "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k" | "%%")
    }) {
        return Err(Unsupported::ComplexExec);
    }
    let first = exec
        .split_whitespace()
        .next()
        .ok_or(Unsupported::MissingExec)?;
    let path = Path::new(first);
    let lower = first.to_ascii_lowercase();
    let base = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if lower.contains("flatpak") || lower.contains("wine") || lower.ends_with(".appimage") {
        return Err(Unsupported::PackageFormat);
    }
    if matches!(
        base,
        "env"
            | "sh"
            | "bash"
            | "zsh"
            | "dash"
            | "fish"
            | "sudo"
            | "pkexec"
            | "nohup"
            | "nice"
            | "setsid"
    ) || ["python", "perl", "ruby", "node", "java"]
        .iter()
        .any(|p| base.starts_with(p))
    {
        return Err(Unsupported::WrapperOrInterpreter);
    }
    if !safe_absolute(path) || first.contains('%') {
        return Err(Unsupported::UnresolvedPath);
    }
    let parent = directory(path.parent().ok_or(Unsupported::UnresolvedPath)?)
        .map_err(|_| Unsupported::UnverifiedExecutable)?;
    let fd = File::from(
        openat(
            &parent,
            path.file_name().ok_or(Unsupported::UnresolvedPath)?,
            READ,
            Mode::empty(),
        )
        .map_err(|_| Unsupported::UnverifiedExecutable)?,
    );
    use std::os::unix::fs::PermissionsExt;
    let meta = fd
        .metadata()
        .map_err(|_| Unsupported::UnverifiedExecutable)?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err(Unsupported::UnverifiedExecutable);
    }
    // Скрипты исключаются; ELF magic не доказывает отсутствие wrapper-логики.
    let mut magic = [0; 4];
    let mut fd = fd;
    fd.read_exact(&mut magic)
        .map_err(|_| Unsupported::UnverifiedExecutable)?;
    if magic != *b"\x7fELF" {
        return Err(Unsupported::WrapperOrInterpreter);
    }
    Ok(path.to_owned())
}
fn stat(text: &str) -> io::Result<(u32, String, u64)> {
    let (pid, rest) = text.split_once(" (").ok_or(io::ErrorKind::InvalidData)?;
    let (name, fields) = rest.rsplit_once(") ").ok_or(io::ErrorKind::InvalidData)?;
    let start = fields
        .split_whitespace()
        .nth(19)
        .ok_or(io::ErrorKind::InvalidData)?
        .parse()
        .map_err(|_| io::ErrorKind::InvalidData)?;
    Ok((
        pid.parse().map_err(|_| io::ErrorKind::InvalidData)?,
        name.to_owned(),
        start,
    ))
}
fn process(root: &File, name: &str, limit: usize) -> io::Result<ProcessCandidate> {
    let dir = File::from(openat(root, name, DIR, Mode::empty())?);
    let before = stat(&read(&dir, "stat", limit)?)?;
    let exe = fs::read_link(fd_path(&dir).join("exe"))
        .ok()
        .filter(|p| safe_absolute(p) && !p.as_os_str().as_encoded_bytes().ends_with(b" (deleted)"));
    let after = stat(&read(&dir, "stat", limit)?)?;
    let exe_after = fs::read_link(fd_path(&dir).join("exe"))
        .ok()
        .filter(|p| safe_absolute(p) && !p.as_os_str().as_encoded_bytes().ends_with(b" (deleted)"));
    if before != after || before.0.to_string() != name || exe != exe_after {
        return Err(io::ErrorKind::NotFound.into());
    }
    Ok(ProcessCandidate {
        pid: before.0,
        name: before.1,
        start_time: before.2,
        exe,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_budget_precedes_child_open() {
        let root = std::env::temp_dir().join(format!("yonik-dir-budget-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for index in 0..12 {
            fs::create_dir(root.join(index.to_string())).unwrap();
        }
        let catalog = Catalog::new(
            vec![root.clone()],
            "/proc".into(),
            "C".into(),
            Limits {
                max_dirs: 1,
                ..Limits::default()
            },
        )
        .unwrap();
        let mut opens = 0;
        let result = catalog
            .desktop_snapshot_with(|dir, name| {
                opens += 1;
                openat(dir, name, DIR, Mode::empty())
            })
            .unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(result.incomplete);
        assert_eq!(
            opens, 0,
            "Бюджет должен проверяться до открытия дочерних каталогов"
        );
    }
}
