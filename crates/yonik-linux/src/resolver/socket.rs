//! Проверка локального endpoint до отправки Varlink-запроса.

use super::ResolverError;
use std::{
    fs::Metadata,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::Path,
};
use tokio::net::UnixStream;

#[derive(Clone, Copy)]
pub(super) enum Endpoint {
    Main,
    Monitor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolverPeer {
    pub uid: u32,
    pub pid: Option<u32>,
    pub socket_device: u64,
    pub socket_inode: u64,
}
pub(super) struct Connected {
    pub stream: UnixStream,
    pub peer: ResolverPeer,
}

impl Endpoint {
    fn path(self) -> &'static Path {
        Path::new(match self {
            Self::Main => "/run/systemd/resolve/io.systemd.Resolve",
            Self::Monitor => "/run/systemd/resolve/io.systemd.Resolve.Monitor",
        })
    }
}

pub(super) async fn connect(
    endpoint: Endpoint,
    cancel: &mut crate::Cancellation,
    deadline: tokio::time::Instant,
) -> Result<Connected, ResolverError> {
    connect_at(endpoint.path(), true, cancel, deadline).await
}

async fn connect_at(
    path: &Path,
    production: bool,
    cancel: &mut crate::Cancellation,
    deadline: tokio::time::Instant,
) -> Result<Connected, ResolverError> {
    cancel.check().map_err(|_| ResolverError::Cancelled)?;
    if tokio::time::Instant::now() >= deadline {
        return Err(ResolverError::Timeout);
    }
    let before = metadata(path, production)?;
    let operation = async {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|_| ResolverError::Unavailable)?;
        let after = metadata(path, production)?;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.uid() != after.uid()
            || before.mode() != after.mode()
        {
            return Err(ResolverError::Untrusted);
        }
        let credentials = stream.peer_cred().map_err(|_| ResolverError::Untrusted)?;
        if !trusted_peer(before.uid(), credentials.uid()) {
            return Err(ResolverError::Untrusted);
        }
        Ok(Connected {
            stream,
            peer: ResolverPeer {
                uid: credentials.uid(),
                pid: credentials
                    .pid()
                    .and_then(|pid| u32::try_from(pid).ok())
                    .filter(|pid| *pid > 0),
                socket_device: before.dev(),
                socket_inode: before.ino(),
            },
        })
    };
    tokio::select! {
        biased;
        _=cancel.cancelled()=>Err(ResolverError::Cancelled),
        result=tokio::time::timeout_at(deadline,operation)=>result.map_err(|_|ResolverError::Timeout)?,
    }
}
fn metadata(path: &Path, production: bool) -> Result<Metadata, ResolverError> {
    if production && path != Endpoint::Main.path() && path != Endpoint::Monitor.path() {
        return Err(ResolverError::Untrusted);
    }
    let socket = std::fs::symlink_metadata(path).map_err(metadata_error)?;
    if !socket.file_type().is_socket() {
        return Err(ResolverError::Untrusted);
    }
    if production {
        for directory in ["/", "/run", "/run/systemd", "/run/systemd/resolve"] {
            let parent = std::fs::symlink_metadata(directory).map_err(metadata_error)?;
            let owner_ok = parent.uid() == 0
                || (directory == "/run/systemd/resolve" && parent.uid() == socket.uid());
            if !parent.file_type().is_dir() || parent.mode() & 0o022 != 0 || !owner_ok {
                return Err(ResolverError::Untrusted);
            }
        }
    } else {
        // Только приватный путь cfg(test); production всегда использует enum endpoint.
        let parent = std::fs::symlink_metadata(path.parent().ok_or(ResolverError::Untrusted)?)
            .map_err(metadata_error)?;
        if !parent.file_type().is_dir()
            || parent.mode() & 0o022 != 0
            || (parent.uid() != 0 && parent.uid() != socket.uid())
        {
            return Err(ResolverError::Untrusted);
        }
    }
    Ok(socket)
}
fn metadata_error(error: std::io::Error) -> ResolverError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ResolverError::Unavailable
    } else {
        ResolverError::Untrusted
    }
}
fn trusted_peer(socket_uid: u32, peer_uid: u32) -> bool {
    peer_uid == 0 || peer_uid == socket_uid
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{DirBuilderExt, symlink},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    use tokio::{
        io::AsyncReadExt,
        net::UnixListener,
        time::{Duration, Instant},
    };
    static NEXT: AtomicU64 = AtomicU64::new(1);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "yonik-resolver-socket-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::DirBuilder::new().mode(0o700).create(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn activation_root_or_service_uid_is_required() {
        assert!(trusted_peer(991, 0));
        assert!(trusted_peer(991, 991));
        assert!(!trusted_peer(991, 1000));
    }
    #[tokio::test]
    async fn trusted_private_socket_connects_without_sending_payload_and_closes() {
        let root = Directory::new();
        let path = root.0.join("resolve");
        let listener = UnixListener::bind(&path).unwrap();
        let (_source, mut cancel) = crate::Cancellation::new();
        let connected = connect_at(
            &path,
            false,
            &mut cancel,
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .expect("Проверенный сокет");
        let (mut peer, _) = listener.accept().await.unwrap();
        assert_eq!(connected.peer.uid, rustix::process::getuid().as_raw());
        assert!(connected.peer.socket_inode > 0);
        drop(connected);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn cancelled_or_expired_connect_does_not_access_endpoint() {
        let (source, mut cancel) = crate::Cancellation::new();
        source.send_replace(true);
        assert_eq!(
            connect_at(
                Path::new("/nonexistent"),
                true,
                &mut cancel,
                Instant::now() + Duration::from_secs(1)
            )
            .await
            .err(),
            Some(ResolverError::Cancelled)
        );
        let (_source, mut cancel) = crate::Cancellation::new();
        assert_eq!(
            connect_at(Path::new("/nonexistent"), true, &mut cancel, Instant::now())
                .await
                .err(),
            Some(ResolverError::Timeout)
        );
    }
    #[tokio::test]
    async fn final_symlink_and_arbitrary_production_path_are_untrusted() {
        let root = Directory::new();
        let path = root.0.join("resolve");
        let _listener = UnixListener::bind(&path).unwrap();
        let alias = root.0.join("alias");
        symlink(&path, &alias).unwrap();
        assert_eq!(
            metadata(&alias, false).err(),
            Some(ResolverError::Untrusted)
        );
        assert_eq!(metadata(&path, true).err(), Some(ResolverError::Untrusted));
    }
}
