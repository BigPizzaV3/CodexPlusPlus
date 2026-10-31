use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use super::media::LocalAttachment;

const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_DIRECTORIES: usize = 100;
const MARKER: &str = ".weixin-media.json";
const LEASE: &str = ".weixin-media.lock";

#[derive(Deserialize, Serialize)]
struct Manifest {
    version: u8,
    directory: String,
    files: Vec<(String, u64, u64)>,
}

// Windows 持有不共享删除权限的父目录；Unix 的读写/删除始终相对目录句柄。
struct Directory {
    path: PathBuf,
    _file: File,
}

impl Directory {
    fn open(path: &Path) -> anyhow::Result<Self> {
        let file = open_path(path, true, false, false)?;
        Ok(Self {
            path: path.into(),
            _file: file,
        })
    }

    fn child(&self, name: &str) -> anyhow::Result<Self> {
        let file = self.open_file(name, true, false)?;
        Ok(Self {
            path: self.path.join(name),
            _file: file,
        })
    }

    fn create_child(&self, name: &str) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let name = std::ffi::CString::new(name)?;
            if unsafe { libc::mkdirat(self._file.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        std::fs::create_dir(self.path.join(name))
    }

    fn open_file(&self, name: &str, directory: bool, create: bool) -> anyhow::Result<File> {
        self.open_file_with_append(name, directory, create, false)
    }

    fn open_file_with_append(
        &self,
        name: &str,
        directory: bool,
        create: bool,
        append: bool,
    ) -> anyhow::Result<File> {
        if !matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) {
            bail!("附件目录包含无效文件名");
        }
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            let name = std::ffi::CString::new(name)?;
            let flags = libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | if directory { libc::O_DIRECTORY } else { 0 }
                | if create {
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
                } else if append {
                    libc::O_WRONLY | libc::O_APPEND
                } else {
                    libc::O_RDONLY
                };
            let fd = unsafe { libc::openat(self._file.as_raw_fd(), name.as_ptr(), flags, 0o600) };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let metadata = file.metadata()?;
            if metadata.is_dir() != directory || (!directory && !metadata.is_file()) {
                bail!("附件目录存在特殊文件");
            }
            Ok(file)
        }
        #[cfg(not(unix))]
        open_path(&self.path.join(name), directory, create, append)
    }

    fn ensure_gitignore(&self) -> anyhow::Result<()> {
        let mut file = match self.open_file(".gitignore", false, true) {
            Ok(mut file) => {
                file.write_all(b"*\n")?;
                file.sync_all()?;
                return Ok(());
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                self.open_file(".gitignore", false, false)?
            }
            Err(error) => return Err(error),
        };
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        if contents.lines().last() != Some("*") {
            // 保留用户已有内容，末尾规则覆盖先前的反向匹配；不跟随链接。
            let mut file = self.open_file_with_append(".gitignore", false, false, true)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if file.metadata()?.nlink() != 1 {
                    bail!("Git 忽略文件不允许硬链接");
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                use windows::Win32::Foundation::HANDLE;
                use windows::Win32::Storage::FileSystem::{
                    BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
                };
                let mut metadata = BY_HANDLE_FILE_INFORMATION::default();
                unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut metadata)? };
                if metadata.nNumberOfLinks != 1 {
                    bail!("Git 忽略文件不允许硬链接");
                }
            }
            file.write_all(b"\n*\n")?;
            file.sync_all()?;
        }
        Ok(())
    }

    fn remove(&self, name: &str, directory: bool) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let name = std::ffi::CString::new(name)?;
            if unsafe {
                libc::unlinkat(
                    self._file.as_raw_fd(),
                    name.as_ptr(),
                    if directory { libc::AT_REMOVEDIR } else { 0 },
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(unix))]
        if directory {
            std::fs::remove_dir(self.path.join(name))?;
        } else {
            std::fs::remove_file(self.path.join(name))?;
        }
        Ok(())
    }

    fn publish(&self, temporary: &str, filename: &str) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let temporary = std::ffi::CString::new(temporary)?;
            let filename = std::ffi::CString::new(filename)?;
            let descriptor = self._file.as_raw_fd();
            if unsafe {
                libc::linkat(
                    descriptor,
                    temporary.as_ptr(),
                    descriptor,
                    filename.as_ptr(),
                    0,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(unix))]
        std::fs::hard_link(self.path.join(temporary), self.path.join(filename))?;
        Ok(())
    }
}

fn open_path(path: &Path, directory: bool, create: bool, append: bool) -> anyhow::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options
        .read(!create && !append)
        .write(create)
        .create_new(create)
        .append(append);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(1 | 2)
            .custom_flags(0x0020_0000 | 0x0200_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            libc::O_NOFOLLOW | libc::O_NONBLOCK | if directory { libc::O_DIRECTORY } else { 0 },
        );
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("附件目录不允许链接或重解析点");
        }
    }
    if metadata.is_dir() != directory || (!directory && !metadata.is_file()) {
        bail!("附件目录存在特殊文件");
    }
    Ok(file)
}

pub(super) struct Inbox {
    // 保持整个目录链的句柄直到下载和登记完成。
    _parents: Vec<Directory>,
    directory: Directory,
    _lease: File,
}

impl Inbox {
    pub(super) fn create(workspace: &Path) -> anyhow::Result<Self> {
        let root = workspace.canonicalize().context("无法读取微信工作目录")?;
        let mut parents = vec![Directory::open(&root)?];
        for name in [".codex-plus", "weixin", "inbox"] {
            let parent = parents.last().unwrap();
            match parent.create_child(name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let directory = parent.child(name).context("微信附件目录不允许链接")?;
            if name == ".codex-plus" {
                directory
                    .ensure_gitignore()
                    .context("无法设置微信附件 Git 忽略保护")?;
            }
            parents.push(directory);
        }
        let inbox = parents.last().unwrap();
        if let Err(error) = cleanup(inbox, SystemTime::now(), MAX_DIRECTORIES - 1) {
            log_cleanup_error(&error);
        }
        let name = uuid::Uuid::new_v4().to_string();
        inbox.create_child(&name)?;
        let directory = inbox.child(&name)?;
        let lease = directory.open_file(LEASE, false, true)?;
        lease.try_lock().context("无法锁定本次微信收件目录")?;
        let result = Self {
            _parents: parents,
            directory,
            _lease: lease,
        };
        result.record(&[])?;
        Ok(result)
    }

    pub(super) fn path(&self) -> &Path {
        &self.directory.path
    }

    pub(super) async fn save(&self, filename: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
        if !matches!(
            Path::new(filename)
                .components()
                .collect::<Vec<_>>()
                .as_slice(),
            [Component::Normal(_)]
        ) || filename.contains(['/', '\\', ':'])
        {
            bail!("附件文件名必须是单个路径组件");
        }
        let filename = format!("{}-{filename}", uuid::Uuid::new_v4().simple());
        let temporary = format!(".{}.part", uuid::Uuid::new_v4().simple());
        let file = self.directory.open_file(&temporary, false, true)?;
        let mut file = tokio::fs::File::from_std(file);
        let saved = async {
            file.write_all(bytes).await?;
            file.flush().await?;
            file.sync_all().await
        }
        .await;
        drop(file);
        if let Err(error) = saved {
            let _ = self.directory.remove(&temporary, false);
            return Err(error).context("无法写入微信附件");
        }
        let published = self.directory.publish(&temporary, &filename);
        let removed = self.directory.remove(&temporary, false);
        published.context("无法原子保存微信附件")?;
        removed.context("无法移除微信附件临时文件")?;
        Ok(self.path().join(filename))
    }

    pub(super) fn record(&self, attachments: &[LocalAttachment]) -> anyhow::Result<()> {
        let mut files = Vec::new();
        for attachment in attachments {
            let parent = attachment.path.parent().context("附件缺少收件目录")?;
            if parent.canonicalize()? != self.path() {
                bail!("附件不属于本次收件目录");
            }
            let name = attachment
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .context("附件文件名无效")?;
            let file = self.directory.open_file(name, false, false)?;
            let metadata = file.metadata()?;
            files.push((name.to_owned(), metadata.len(), modified_nanos(&metadata)?));
        }
        let marker = Manifest {
            version: 1,
            directory: self
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            files,
        };
        // 已有标记只在本次持有目录链期间更新；清理从不推断旧目录的归属。
        if self.directory.open_file(MARKER, false, false).is_ok() {
            self.directory.remove(MARKER, false)?;
        }
        let mut file = self.directory.open_file(MARKER, false, true)?;
        file.write_all(&serde_json::to_vec(&marker)?)?;
        file.sync_all()?;
        Ok(())
    }
}

fn modified_nanos(metadata: &std::fs::Metadata) -> anyhow::Result<u64> {
    Ok(metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos()
        .try_into()?)
}

fn owned_manifest(directory: &Directory, name: &str) -> anyhow::Result<(Manifest, SystemTime)> {
    if uuid::Uuid::parse_str(name).is_err() {
        bail!("不是微信生成的目录");
    }
    let file = directory.open_file(MARKER, false, false)?;
    let metadata = file.metadata()?;
    if metadata.len() > 64 * 1024 {
        bail!("收件目录标记过大");
    }
    let marker: Manifest = serde_json::from_reader(file.take(64 * 1024 + 1))?;
    if marker.version != 1 || marker.directory != name || marker.files.len() > 10 {
        bail!("收件目录标记无效");
    }
    let entries = std::fs::read_dir(&directory.path)?.collect::<Result<Vec<_>, _>>()?;
    if entries.len() != marker.files.len() + 2 {
        bail!("收件目录含有未登记内容");
    }
    let mut names = std::collections::HashSet::new();
    for (name, size, modified) in &marker.files {
        if name == MARKER || name == LEASE || !names.insert(name) {
            bail!("收件标记包含重复文件");
        }
        let file = directory.open_file(name, false, false)?;
        let metadata = file.metadata()?;
        if metadata.len() != *size || modified_nanos(&metadata)? != *modified {
            bail!("收件文件已被修改");
        }
    }
    Ok((marker, metadata.modified()?))
}

fn cleanup(inbox: &Directory, now: SystemTime, retain: usize) -> anyhow::Result<()> {
    let mut owned = Vec::new();
    for entry in std::fs::read_dir(&inbox.path)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log_cleanup_error(&error.into());
                continue;
            }
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(directory) = inbox.child(&name) else {
            continue;
        };
        let Ok(lease) = directory.open_file(LEASE, false, false) else {
            continue;
        };
        if lease.try_lock().is_err() {
            continue;
        }
        if let Ok((_, modified)) = owned_manifest(&directory, &name) {
            owned.push((modified, name));
        }
    }
    owned.sort();
    let excess = owned.len().saturating_sub(retain);
    for (index, (modified, name)) in owned.into_iter().enumerate() {
        if index >= excess && now.duration_since(modified).unwrap_or_default() <= RETENTION {
            continue;
        }
        // 单个旧目录清理失败不能阻断其他清理或本次收件。
        if let Err(error) = cleanup_directory(inbox, &name, modified) {
            log_cleanup_error(&error);
        }
    }
    Ok(())
}

fn cleanup_directory(inbox: &Directory, name: &str, modified: SystemTime) -> anyhow::Result<()> {
    let directory = inbox.child(name)?;
    let Ok(lease) = directory.open_file(LEASE, false, false) else {
        return Ok(());
    };
    if lease.try_lock().is_err() {
        return Ok(());
    }
    let Ok((marker, current_modified)) = owned_manifest(&directory, name) else {
        return Ok(());
    };
    if current_modified != modified {
        return Ok(());
    }
    for (filename, _, _) in marker.files {
        directory.remove(&filename, false)?;
    }
    directory.remove(MARKER, false)?;
    drop(lease);
    directory.remove(LEASE, false)?;
    drop(directory);
    // 仅移除空目录；额外内容永不递归删除。
    inbox.remove(name, true)
}

fn log_cleanup_error(error: &anyhow::Error) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "weixin.inbox.cleanup_skipped",
        serde_json::json!({ "error": format!("{error:#}") }),
    );
}

#[cfg(test)]
mod tests {
    use super::super::media::AttachmentKind;
    use super::*;

    #[tokio::test]
    async fn atomic_save_and_record_keep_unique_complete_files_without_temporary_entries() {
        let root = tempfile::tempdir().unwrap();
        let inbox = Inbox::create(root.path()).unwrap();
        let payload = vec![42; 128 * 1024];
        let first = inbox.save("report.txt", &payload).await.unwrap();
        let second = inbox.save("report.txt", b"second").await.unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), payload);
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        let attachments = [first, second].map(|path| LocalAttachment {
            path,
            kind: AttachmentKind::File,
        });
        inbox.record(&attachments).unwrap();
        let (marker, _) = owned_manifest(
            &inbox.directory,
            inbox.path().file_name().unwrap().to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(marker.files.len(), 2);
        assert_eq!(std::fs::read_dir(inbox.path()).unwrap().count(), 4);
        assert!(std::fs::read_dir(inbox.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        }));
    }

    #[tokio::test]
    async fn rejects_path_components_and_recording_files_from_other_directories() {
        let root = tempfile::tempdir().unwrap();
        let inbox = Inbox::create(root.path()).unwrap();
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "sub/file",
            "sub\\file",
            "C:escape",
        ] {
            assert!(inbox.save(name, b"invalid").await.is_err(), "{name}");
        }
        let outside = root.path().join("outside.txt");
        std::fs::write(&outside, b"keep").unwrap();
        assert!(
            inbox
                .record(&[LocalAttachment {
                    path: outside.clone(),
                    kind: AttachmentKind::File,
                }])
                .is_err()
        );
        assert_eq!(std::fs::read(outside).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(inbox.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn record_accepts_canonical_alias_of_the_same_inbox() {
        let root = tempfile::tempdir().unwrap();
        let inbox = Inbox::create(root.path()).unwrap();
        let saved = inbox.save("file.txt", b"owned").await.unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(inbox.path(), &alias).unwrap();
        inbox
            .record(&[LocalAttachment {
                path: alias.join(saved.file_name().unwrap()),
                kind: AttachmentKind::File,
            }])
            .unwrap();
        assert!(
            owned_manifest(
                &inbox.directory,
                inbox.path().file_name().unwrap().to_str().unwrap(),
            )
            .is_ok()
        );
    }

    #[test]
    fn count_limit_includes_the_new_directory_and_preserves_unowned_content() {
        let root = tempfile::tempdir().unwrap();
        let first = Inbox::create(root.path()).unwrap();
        let inbox = first.path().parent().unwrap().to_path_buf();
        let foreign = inbox.join("user-files");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(foreign.join("keep.txt"), b"keep").unwrap();
        let unmarked = inbox.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&unmarked).unwrap();
        std::fs::write(unmarked.join("keep.txt"), b"keep").unwrap();
        drop(first);
        for _ in 0..MAX_DIRECTORIES + 1 {
            drop(Inbox::create(root.path()).unwrap());
        }
        let owned = std::fs::read_dir(&inbox)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join(MARKER).is_file())
            .count();
        assert_eq!(owned, MAX_DIRECTORIES);
        assert_eq!(std::fs::read(foreign.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(std::fs::read(unmarked.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn ttl_boundary_and_modified_or_foreign_content_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let managed = Inbox::create(root.path()).unwrap();
        let path = managed.path().to_path_buf();
        let file = path.join("attachment.bin");
        std::fs::write(&file, b"owned").unwrap();
        managed
            .record(&[LocalAttachment {
                path: file.clone(),
                kind: AttachmentKind::Image,
            }])
            .unwrap();
        let inbox_path = managed.path().parent().unwrap().to_path_buf();
        drop(managed);
        let inbox = Directory::open(&inbox_path).unwrap();
        let modified = std::fs::metadata(path.join(MARKER))
            .unwrap()
            .modified()
            .unwrap();
        cleanup(&inbox, modified + RETENTION, MAX_DIRECTORIES).unwrap();
        assert!(file.exists());
        std::fs::write(path.join("keep.txt"), b"foreign").unwrap();
        cleanup(
            &inbox,
            modified + RETENTION + Duration::from_secs(1),
            MAX_DIRECTORIES,
        )
        .unwrap();
        assert!(file.exists() && path.join("keep.txt").exists());
        // 未登记文件移出后，已修改附件仍不能被删除。
        std::fs::rename(path.join("keep.txt"), root.path().join("keep.txt")).unwrap();
        let original_time = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::fs::write(&file, b"other").unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new().set_modified(original_time + Duration::from_secs(1)),
            )
            .unwrap();
        cleanup(
            &inbox,
            modified + RETENTION + Duration::from_secs(1),
            MAX_DIRECTORIES,
        )
        .unwrap();
        assert!(file.exists());
        std::fs::write(&file, b"owned").unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original_time))
            .unwrap();
        cleanup(
            &inbox,
            modified + RETENTION + Duration::from_secs(1),
            MAX_DIRECTORIES,
        )
        .unwrap();
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(root.path().join("keep.txt")).unwrap(),
            b"foreign"
        );
    }

    #[test]
    fn malformed_marker_and_nested_content_are_never_deleted() {
        let root = tempfile::tempdir().unwrap();
        let managed = Inbox::create(root.path()).unwrap();
        std::fs::create_dir(managed.path().join("user-subdir")).unwrap();
        std::fs::write(managed.path().join("user-subdir/keep"), b"keep").unwrap();
        let inbox_path = managed.path().parent().unwrap().to_path_buf();
        let path = managed.path().to_path_buf();
        drop(managed);
        cleanup(
            &Directory::open(&inbox_path).unwrap(),
            SystemTime::now() + RETENTION * 2,
            0,
        )
        .unwrap();
        assert!(path.join("user-subdir/keep").exists());
        std::fs::write(path.join(MARKER), b"invalid").unwrap();
        cleanup(
            &Directory::open(&inbox_path).unwrap(),
            SystemTime::now() + RETENTION * 2,
            0,
        )
        .unwrap();
        assert!(path.join("user-subdir/keep").exists());
    }

    #[tokio::test]
    async fn cleanup_skips_a_directory_until_the_inbound_turn_releases_it() {
        let root = tempfile::tempdir().unwrap();
        let active = Inbox::create(root.path()).unwrap();
        let path = active.path().to_path_buf();
        let file = active.save("input.txt", b"in use").await.unwrap();
        active
            .record(&[LocalAttachment {
                path: file.clone(),
                kind: AttachmentKind::File,
            }])
            .unwrap();
        let parent = Directory::open(path.parent().unwrap()).unwrap();
        cleanup(&parent, SystemTime::now() + RETENTION * 2, 0).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"in use");
        drop(active);
        cleanup(&parent, SystemTime::now() + RETENTION * 2, 0).unwrap();
        assert!(!path.exists());
    }

    #[cfg(windows)]
    async fn check_delete_locked_attachment_does_not_block_inbound(expired: bool) {
        use std::os::windows::fs::OpenOptionsExt;

        let root = tempfile::tempdir().unwrap();
        let old = Inbox::create(root.path()).unwrap();
        let old_path = old.path().to_path_buf();
        let file = old.save("locked.txt", b"old attachment").await.unwrap();
        old.record(&[LocalAttachment {
            path: file.clone(),
            kind: AttachmentKind::File,
        }])
        .unwrap();
        let modified = SystemTime::now()
            - if expired {
                RETENTION * 2
            } else {
                Duration::from_secs(60)
            };
        File::options()
            .write(true)
            .open(old_path.join(MARKER))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        drop(old);
        // 可共享读写但不共享删除，真实复现 Windows 附件被占用。
        let held = File::options()
            .read(true)
            .share_mode(1 | 2)
            .open(&file)
            .unwrap();
        assert_eq!(
            std::fs::remove_file(&file).unwrap_err().raw_os_error(),
            Some(32)
        );
        if !expired {
            for _ in 1..MAX_DIRECTORIES {
                drop(Inbox::create(root.path()).unwrap());
            }
        }
        let new = Inbox::create(root.path()).unwrap();
        let saved = new.save("new.txt", b"new attachment").await.unwrap();
        new.record(&[LocalAttachment {
            path: saved.clone(),
            kind: AttachmentKind::File,
        }])
        .unwrap();
        assert_eq!(std::fs::read(&saved).unwrap(), b"new attachment");
        assert_eq!(std::fs::read(&file).unwrap(), b"old attachment");
        let parent = Directory::open(old_path.parent().unwrap()).unwrap();
        cleanup(&parent, SystemTime::now() + RETENTION * 2, 0).unwrap();
        assert!(old_path.join(MARKER).is_file());
        assert!(old_path.join(LEASE).is_file());
        // 被占用目录不妨碍其他已归属旧目录清理，活动新目录继续保留。
        assert_eq!(std::fs::read_dir(&parent.path).unwrap().count(), 2);
        drop(held);
        cleanup(&parent, SystemTime::now() + RETENTION * 2, 0).unwrap();
        assert!(!old_path.exists());
        assert_eq!(std::fs::read(saved).unwrap(), b"new attachment");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn expired_delete_locked_attachment_does_not_block_new_inbound() {
        check_delete_locked_attachment_does_not_block_inbound(true).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn excess_delete_locked_attachment_does_not_block_new_inbound() {
        check_delete_locked_attachment_does_not_block_inbound(false).await;
    }

    #[cfg(windows)]
    #[test]
    fn delete_locked_marker_lease_or_directory_does_not_block_new_inbound() {
        for name in [MARKER, LEASE, "."] {
            let root = tempfile::tempdir().unwrap();
            let old = Inbox::create(root.path()).unwrap();
            let old_path = old.path().to_path_buf();
            drop(old);
            let parent = Directory::open(old_path.parent().unwrap()).unwrap();
            let held = open_path(&old_path.join(name), name == ".", false, false).unwrap();
            cleanup(&parent, SystemTime::now() + RETENTION * 2, 0).unwrap();
            assert!(old_path.exists(), "{name}");
            let new = Inbox::create(root.path()).unwrap();
            assert_ne!(new.path(), old_path);
            drop(held);
        }
    }

    #[test]
    fn inbox_creation_still_reports_invalid_workspace_and_directory_errors() {
        let root = tempfile::tempdir().unwrap();
        assert!(Inbox::create(&root.path().join("missing")).is_err());
        let parent = root.path().join(".codex-plus/weixin");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::write(parent.join("inbox"), b"not a directory").unwrap();
        assert!(Inbox::create(root.path()).is_err());
        assert_eq!(
            std::fs::read(parent.join("inbox")).unwrap(),
            b"not a directory"
        );
    }

    fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(["-c", "core.excludesFile=", "-c", "core.autocrlf=false"])
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    #[tokio::test]
    async fn git_ignores_inbound_files_and_preserves_existing_ignore_content() {
        for original in [
            None,
            Some("# custom\n!weixin/\n!weixin/**"),
            Some("# custom\r\n*\r\n"),
        ] {
            let root = tempfile::tempdir().unwrap();
            git(root.path(), &["init", "-q"]);
            let ignore = root.path().join(".codex-plus/.gitignore");
            if let Some(original) = original {
                std::fs::create_dir(ignore.parent().unwrap()).unwrap();
                std::fs::write(&ignore, original).unwrap();
            }
            std::fs::write(root.path().join("public.txt"), b"public").unwrap();
            let inbox = Inbox::create(root.path()).unwrap();
            let attachment = inbox
                .save("private.txt", b"private attachment")
                .await
                .unwrap();
            inbox
                .record(&[LocalAttachment {
                    path: attachment.clone(),
                    kind: AttachmentKind::File,
                }])
                .unwrap();
            let contents = std::fs::read_to_string(&ignore).unwrap();
            if let Some(original) = original {
                assert!(contents.starts_with(original));
            }
            assert_eq!(contents.lines().last(), Some("*"));
            let workspace = root.path().canonicalize().unwrap();
            let relative = attachment
                .strip_prefix(workspace)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let ignored = git(root.path(), &["check-ignore", "-v", "--", &relative]);
            assert!(ignored.starts_with(".codex-plus/.gitignore:"), "{ignored}");
            assert!(ignored.contains(":*\t"), "{ignored}");
            assert_eq!(
                git(
                    root.path(),
                    &["status", "--porcelain", "--untracked-files=all"]
                ),
                "?? public.txt\n"
            );
            git(root.path(), &["add", "-A"]);
            assert_eq!(
                git(root.path(), &["diff", "--cached", "--name-only"]),
                "public.txt\n"
            );
            assert_eq!(
                git(
                    root.path(),
                    &["status", "--porcelain", "--untracked-files=all"]
                ),
                "A  public.txt\n"
            );
            drop(Inbox::create(root.path()).unwrap());
            assert_eq!(std::fs::read_to_string(ignore).unwrap(), contents);
        }
    }

    #[test]
    fn inbox_rejects_directory_or_hardlink_at_gitignore_before_creating_inbox() {
        for hardlink in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let parent = root.path().join(".codex-plus");
            std::fs::create_dir(&parent).unwrap();
            let outside = root.path().join("outside.txt");
            std::fs::write(&outside, b"keep").unwrap();
            let ignore = parent.join(".gitignore");
            if hardlink {
                std::fs::hard_link(&outside, &ignore).unwrap();
            } else {
                std::fs::create_dir(&ignore).unwrap();
            }
            assert!(Inbox::create(root.path()).is_err());
            assert_eq!(std::fs::read(outside).unwrap(), b"keep");
            assert!(!parent.join("weixin").exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn inbox_rejects_gitignore_write_sharing_conflict_before_creating_inbox() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join(".codex-plus");
        std::fs::create_dir(&parent).unwrap();
        let ignore = parent.join(".gitignore");
        std::fs::write(&ignore, b"# keep").unwrap();
        let _held = File::options()
            .read(true)
            .share_mode(1)
            .open(&ignore)
            .unwrap();
        assert!(Inbox::create(root.path()).is_err());
        assert_eq!(std::fs::read(ignore).unwrap(), b"# keep");
        assert!(!parent.join("weixin").exists());
    }

    #[test]
    fn inbox_rejects_linked_gitignore_before_creating_inbox() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep"), b"keep").unwrap();
        let parent = root.path().join(".codex-plus");
        std::fs::create_dir(&parent).unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(parent.join(".gitignore"))
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path().join("keep"), parent.join(".gitignore")).unwrap();
        assert!(Inbox::create(root.path()).is_err());
        assert_eq!(std::fs::read(outside.path().join("keep")).unwrap(), b"keep");
        assert!(!parent.join("weixin").exists());
    }

    #[test]
    fn inbox_rejects_linked_ancestors_before_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep"), b"keep").unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(root.path().join(".codex-plus"))
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), root.path().join(".codex-plus")).unwrap();
        assert!(Inbox::create(root.path()).is_err());
        assert_eq!(std::fs::read(outside.path().join("keep")).unwrap(), b"keep");
        assert!(!outside.path().join("weixin").exists());
    }
}
