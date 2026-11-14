//! Закрытый PID1 lookup leaf: stdin-only канал, fixed image, без TUN и capabilities.
use super::*;
use sha2::{Digest, Sha256};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::fs::FileExt;

const IMAGE: &str = "/usr/lib/yonik/yonik-worker";

struct LeafIdentity {
    unit: String,
    pid: u32,
    uid: u32,
    gid: u32,
    start: u64,
    invocation: [u8; 16],
    pidfd: OwnedFd,
    image: File,
    image_sha256: [u8; 32],
    image_stamp: ImageStamp,
}
/// Не имеет raw constructor: только canonical PID1 launch выдаёт owning lease.
pub(crate) struct LookupLeafLease(Arc<LeafIdentity>);
pub(crate) struct VerifiedLookupLeaf {
    channel: OwnedFd,
    lease: LookupLeafLease,
}
impl VerifiedLookupLeaf {
    pub(crate) fn into_parts(self) -> (OwnedFd, LookupLeafLease) {
        (self.channel, self.lease)
    }
}
impl LookupLeafLease {
    pub(crate) fn pid(&self) -> u32 {
        self.0.pid
    }
    pub(crate) fn uid(&self) -> u32 {
        self.0.uid
    }
    pub(crate) fn start(&self) -> u64 {
        self.0.start
    }
    pub(crate) fn invocation(&self) -> [u8; 16] {
        self.0.invocation
    }
    pub(crate) fn verify_image(&self, image: &File) -> Result<()> {
        same_image(&self.0.image, image)?;
        if image_stamp(image)? != self.0.image_stamp || image_hash(image)? != self.0.image_sha256 {
            return Err(WorkerSupervisorError::Stale);
        }
        Ok(())
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.0.validate()
    }
}
impl LeafIdentity {
    fn validate(&self) -> Result<()> {
        if bootstrap_start(self.pid)? != self.start {
            return Err(WorkerSupervisorError::Stale);
        }
        let info = read_proc(&format!("/proc/self/fdinfo/{}", self.pidfd.as_raw_fd()))?;
        if info.lines().find_map(|s| {
            s.strip_prefix("Pid:")
                .and_then(|v| v.trim().parse::<u32>().ok())
        }) != Some(self.pid)
        {
            return Err(WorkerSupervisorError::Stale);
        }
        validate_status(
            &read_proc(&format!("/proc/{}/status", self.pid))?,
            self.uid,
            self.gid,
        )?;
        if read_proc(&format!("/proc/{}/cgroup", self.pid))?
            != format!("0::/system.slice/{}\n", self.unit)
        {
            return Err(WorkerSupervisorError::Stale);
        }
        same_image(&self.image, &installed_image()?)?;
        if image_stamp(&self.image)? != self.image_stamp {
            return Err(WorkerSupervisorError::Stale);
        }
        if bootstrap_start(self.pid)? != self.start {
            return Err(WorkerSupervisorError::Stale);
        }
        Ok(())
    }
}
fn read_proc(path: &str) -> Result<String> {
    read_bounded(File::open(path).map_err(unavailable)?)
}
type ImageStamp = (u64, u64, u64, i64, i64, i64, i64);
fn image_stamp(file: &File) -> Result<ImageStamp> {
    let m = file.metadata().map_err(unavailable)?;
    Ok((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}
fn image_hash(file: &File) -> Result<[u8; 32]> {
    immutable(file)?;
    let before = image_stamp(file)?;
    if before.2 == 0 || before.2 > 128 * 1024 * 1024 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let mut digest = Sha256::new();
    let mut offset = 0;
    let mut bytes = [0; 65536];
    while offset < before.2 {
        let n = file.read_at(&mut bytes, offset).map_err(unavailable)?;
        if n == 0 {
            return Err(WorkerSupervisorError::Stale);
        }
        digest.update(&bytes[..n]);
        offset += n as u64;
        if offset > before.2 {
            return Err(WorkerSupervisorError::Stale);
        }
    }
    if image_stamp(file)? != before {
        return Err(WorkerSupervisorError::Stale);
    }
    Ok(digest.finalize().into())
}
fn immutable(file: &File) -> Result<()> {
    trusted(file, false)?;
    if file.metadata().map_err(unavailable)?.mode() & 0o222 != 0 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(())
}
fn installed_image() -> Result<File> {
    let mut dir = root()?;
    for component in ["usr", "lib", "yonik"] {
        dir = child(&dir, component)?;
    }
    let image =
        File::from(openat(&dir, "yonik-worker", FILE_FLAGS, Mode::empty()).map_err(unavailable)?);
    immutable(&image)?;
    Ok(image)
}
fn same_image(expected: &File, actual: &File) -> Result<()> {
    immutable(expected)?;
    immutable(actual)?;
    let a = expected.metadata().map_err(unavailable)?;
    let b = actual.metadata().map_err(unavailable)?;
    if (
        a.dev(),
        a.ino(),
        a.len(),
        a.mtime(),
        a.mtime_nsec(),
        a.ctime(),
        a.ctime_nsec(),
    ) != (
        b.dev(),
        b.ino(),
        b.len(),
        b.mtime(),
        b.mtime_nsec(),
        b.ctime(),
        b.ctime_nsec(),
    ) {
        return Err(WorkerSupervisorError::Stale);
    }
    Ok(())
}
fn field<'a>(status: &'a str, key: &str) -> Result<&'a str> {
    let mut fields = status.lines().filter_map(|line| line.strip_prefix(key));
    let value = fields
        .next()
        .ok_or(WorkerSupervisorError::Untrusted)?
        .trim();
    if fields.next().is_some() {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(value)
}
fn validate_status(status: &str, uid: u32, gid: u32) -> Result<()> {
    if uid == 0 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    for (name, id) in [("Uid:", uid), ("Gid:", gid)] {
        let values = field(status, name)?
            .split_whitespace()
            .map(str::parse::<u32>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(unavailable)?;
        if values != [id; 4] {
            return Err(WorkerSupervisorError::Untrusted);
        }
    }
    for name in ["CapInh:", "CapPrm:", "CapEff:", "CapBnd:", "CapAmb:"] {
        if u64::from_str_radix(field(status, name)?, 16).map_err(unavailable)? != 0 {
            return Err(WorkerSupervisorError::Untrusted);
        }
    }
    if field(status, "NoNewPrivs:")? != "1" {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(())
}
fn primary_gid(uid: u32) -> Result<u32> {
    let mut directory = root()?;
    directory = child(&directory, "etc")?;
    let file =
        File::from(openat(&directory, "passwd", FILE_FLAGS, Mode::empty()).map_err(unavailable)?);
    trusted(&file, false)?;
    parse_primary_gid(&read_bounded(file)?, uid)
}
fn parse_primary_gid(passwd: &str, uid: u32) -> Result<u32> {
    let mut found = None;
    for line in passwd.lines() {
        let parts: Vec<_> = line.split(':').collect();
        if parts.len() != 7 {
            return Err(WorkerSupervisorError::Untrusted);
        }
        if parts[2].parse::<u32>().map_err(unavailable)? == uid {
            let gid = parts[3].parse::<u32>().map_err(unavailable)?;
            if found.replace(gid).is_some() || gid == 0 {
                return Err(WorkerSupervisorError::Untrusted);
            }
        }
    }
    found.ok_or(WorkerSupervisorError::Unavailable)
}
fn leaf_empty(name: &str) -> Result<bool> {
    let mut directory = root()?;
    for component in ["sys", "fs", "cgroup"] {
        directory = child(&directory, component)?;
    }
    if fstatfs(&directory).map_err(unavailable)?.f_type as u64 != 0x6367_7270 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let directory = child(&directory, "system.slice")?;
    let group = match openat(&directory, name, DIR_FLAGS, Mode::empty()) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(true),
        Err(error) => return Err(unavailable(error)),
    };
    trusted(&group, true)?;
    empty_events(&read_bounded(File::from(
        openat(&group, "cgroup.events", FILE_FLAGS, Mode::empty()).map_err(unavailable)?,
    ))?)
}

pub(super) struct LeafPending {
    unit: String,
    uid: u32,
    gid: u32,
    _guard: RunRevocationGuard,
    identity: Option<Arc<LeafIdentity>>,
}
impl SystemWorkerSupervisor {
    pub(crate) async fn launch_lookup_leaf(
        &mut self,
        owner: &RunAuthority,
        authorization: &RunAuthorization,
        cancel: &mut Cancellation,
        deadline: Instant,
    ) -> Result<VerifiedLookupLeaf> {
        authorization.validate_owner(owner).map_err(unavailable)?;
        if self.lookup_leaf.is_some() {
            return Err(WorkerSupervisorError::Busy);
        }
        let (nonce, _, _) = authorization.challenge_context().map_err(unavailable)?;
        let name = leaf_unit(nonce);
        let uid = authorization
            .owner_binding()
            .map_err(unavailable)?
            .owner_uid();
        if uid == 0 {
            return Err(WorkerSupervisorError::Untrusted);
        }
        let gid = primary_gid(uid)?;
        let image = installed_image()?;
        let image_stamp = image_stamp(&image)?;
        let image_sha256 = image_hash(&image)?;
        let (helper, leaf) = rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .map_err(unavailable)?;
        rustix::net::sockopt::set_socket_passcred(&helper, true).map_err(unavailable)?;
        rustix::net::sockopt::set_socket_passcred(&leaf, true).map_err(unavailable)?;
        let properties = transient_properties(uid, gid, &leaf);
        self.lookup_leaf = Some(LeafPending {
            unit: name.clone(),
            uid,
            gid,
            _guard: authorization.guard(),
            identity: None,
        });
        let auxiliary: Vec<(&str, Vec<(&str, zbus::zvariant::Value<'_>)>)> = Vec::new();
        let mut foreign_unit = false;
        let launch=bounded(cancel, deadline, async {
            let result=self.connection
                .call_method(
                    Some(self.owner.as_str()),
                    MANAGER,
                    Some("org.freedesktop.systemd1.Manager"),
                    "StartTransientUnit",
                    &(&name, "fail", properties, auxiliary),
                )
                .await;
            if matches!(&result,Err(zbus::Error::MethodError(name,_,_)) if name.as_str()=="org.freedesktop.systemd1.UnitExists") {foreign_unit=true;}
            result.map_err(unavailable)?;
            Ok(())
        }).await;
        if foreign_unit {
            self.lookup_leaf = None;
        }
        launch?;
        drop(leaf);
        loop {
            authorization.validate_owner(owner).map_err(unavailable)?;
            let unit = bounded(cancel, deadline, self.properties(&name, UNIT_IFACE)).await?;
            let service = bounded(cancel, deadline, self.properties(&name, SERVICE_IFACE)).await?;
            let pid = number32(&service, "MainPID")?;
            if native_activation_ready(&string(&unit, "ActiveState")?, pid)? {
                validate_properties(&unit, &service, uid, gid, &name, false)?;
                let pid_value = i32::try_from(pid)
                    .ok()
                    .and_then(rustix::process::Pid::from_raw)
                    .ok_or(WorkerSupervisorError::Untrusted)?;
                let invocation: [u8; 16] = array(&unit, "InvocationID")?
                    .try_into()
                    .map_err(unavailable)?;
                if invocation == [0; 16] {
                    return Err(WorkerSupervisorError::Untrusted);
                }
                let identity = Arc::new(LeafIdentity {
                    unit: name.clone(),
                    pid,
                    uid,
                    gid,
                    start: bootstrap_start(pid)?,
                    invocation,
                    pidfd: rustix::process::pidfd_open(
                        pid_value,
                        rustix::process::PidfdFlags::NONBLOCK,
                    )
                    .map_err(unavailable)?,
                    image,
                    image_stamp,
                    image_sha256,
                });
                identity.validate()?;
                let after = bounded(cancel, deadline, self.properties(&name, UNIT_IFACE)).await?;
                let after_service =
                    bounded(cancel, deadline, self.properties(&name, SERVICE_IFACE)).await?;
                if array(&after, "InvocationID")? != invocation
                    || number32(&after_service, "MainPID")? != pid
                {
                    return Err(WorkerSupervisorError::Stale);
                }
                authorization.validate_owner(owner).map_err(unavailable)?;
                self.lookup_leaf
                    .as_mut()
                    .ok_or(WorkerSupervisorError::Stale)?
                    .identity = Some(Arc::clone(&identity));
                return Ok(VerifiedLookupLeaf {
                    channel: helper,
                    lease: LookupLeafLease(identity),
                });
            }
            bounded(cancel, deadline, async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok(())
            })
            .await?;
        }
    }
    pub(super) async fn stop_lookup_leaf(
        &mut self,
        cancel: &mut Cancellation,
        deadline: Instant,
    ) -> Result<()> {
        if self.lookup_leaf.is_none() {
            return Ok(());
        }
        let name = self
            .lookup_leaf
            .as_ref()
            .ok_or(WorkerSupervisorError::Stale)?
            .unit
            .clone();
        // Ожидаемый fixed cgroup — единственный terminal fence. Неизвестность хранит guard.
        match bounded(cancel, deadline, self.properties(&name, UNIT_IFACE)).await {
            Ok(unit) => {
                let service =
                    bounded(cancel, deadline, self.properties(&name, SERVICE_IFACE)).await?;
                let pending = self
                    .lookup_leaf
                    .as_ref()
                    .ok_or(WorkerSupervisorError::Stale)?;
                validate_properties(
                    &unit,
                    &service,
                    pending.uid,
                    pending.gid,
                    &name,
                    matches!(
                        string(&unit, "ActiveState")?.as_str(),
                        "inactive" | "failed"
                    ) && leaf_empty(&name)?,
                )?;
                if let Some(identity) = pending.identity.as_ref() {
                    let invocation = array(&unit, "InvocationID")?;
                    if invocation != identity.invocation {
                        return Err(WorkerSupervisorError::Stale);
                    }
                }
                bounded(cancel, deadline, self.unit_job("StopUnit", &name)).await?;
            }
            Err(error) => {
                if !self.lookup_absent(&name, cancel, deadline).await? {
                    return Err(error);
                }
            }
        }
        loop {
            if leaf_empty(&name)? {
                self.lookup_leaf = None;
                return Ok(());
            }
            bounded(cancel, deadline, async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok(())
            })
            .await?;
        }
    }
    async fn lookup_absent(
        &self,
        name: &str,
        cancel: &mut Cancellation,
        deadline: Instant,
    ) -> Result<bool> {
        bounded(cancel, deadline, async {
            match self
                .connection
                .call_method(
                    Some(self.owner.as_str()),
                    MANAGER,
                    Some("org.freedesktop.systemd1.Manager"),
                    "GetUnit",
                    &(name,),
                )
                .await
            {
                Ok(_) => Ok(false),
                Err(zbus::Error::MethodError(name, _, _))
                    if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
                {
                    Ok(true)
                }
                Err(error) => Err(unavailable(error)),
            }
        })
        .await
    }
}
fn leaf_unit(nonce: [u8; 32]) -> String {
    use std::fmt::Write;
    let mut name = String::from("yonik-native-lookup-");
    for byte in nonce {
        let _ = write!(&mut name, "{byte:02x}");
    }
    name.push_str(".service");
    name
}
fn transient_properties(
    uid: u32,
    gid: u32,
    stdin: &OwnedFd,
) -> Vec<(&'static str, zbus::zvariant::Value<'_>)> {
    use zbus::zvariant::{Fd, Value};
    vec![
        (
            "Description",
            Value::from("Yonik — закрытый application lookup leaf"),
        ),
        ("Type", Value::from("exec")),
        ("User", Value::from(uid.to_string())),
        ("Group", Value::from(gid.to_string())),
        (
            "ExecStart",
            Value::from(vec![(IMAGE, vec![IMAGE, "--lookup"], false)]),
        ),
        (
            "StandardInputFileDescriptor",
            Value::from(Fd::from(stdin.as_fd())),
        ),
        ("StandardOutput", Value::from("null")),
        ("StandardError", Value::from("null")),
        ("CapabilityBoundingSet", Value::from(0u64)),
        ("AmbientCapabilities", Value::from(0u64)),
        ("NoNewPrivileges", Value::from(true)),
        ("DevicePolicy", Value::from("closed")),
        ("RestrictNamespaces", Value::from(0u64)),
        ("RestrictSUIDSGID", Value::from(true)),
        ("KillMode", Value::from("control-group")),
        ("SendSIGKILL", Value::from(true)),
        ("TimeoutStopUSec", Value::from(5_000_000u64)),
        ("Restart", Value::from("no")),
        ("CollectMode", Value::from("inactive-or-failed")),
        ("BindsTo", Value::from(vec!["yonik-helper.service"])),
        ("After", Value::from(vec!["yonik-helper.service"])),
        ("Slice", Value::from("system.slice")),
    ]
}
fn validate_properties(
    unit: &Properties,
    service: &Properties,
    uid: u32,
    gid: u32,
    name: &str,
    allow_empty: bool,
) -> Result<()> {
    let group = string(unit, "ControlGroup")?;
    if !boolean(unit, "Transient")?
        || string(unit, "Id")? != name
        || !(group == format!("/system.slice/{name}") || allow_empty && group.is_empty())
        || string(service, "User")? != uid.to_string()
        || string(service, "Group")? != gid.to_string()
        || number64(service, "CapabilityBoundingSet")? != 0
        || number64(service, "AmbientCapabilities")? != 0
        || !boolean(service, "NoNewPrivileges")?
        || string(service, "KillMode")? != "control-group"
        || string(service, "DevicePolicy")? != "closed"
        || string(service, "Type")? != "exec"
    {
        return Err(WorkerSupervisorError::Untrusted);
    }
    let exec: ExecCommands = service
        .get("ExecStart")
        .ok_or(WorkerSupervisorError::Untrusted)?
        .try_clone()
        .map_err(unavailable)?
        .try_into()
        .map_err(unavailable)?;
    if exec.len() != 1 || exec[0].0 != IMAGE || exec[0].1 != [IMAGE, "--lookup"] || exec[0].2 {
        return Err(WorkerSupervisorError::Untrusted);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn status(uid: u32, gid: u32) -> String {
        format!(
            "Uid:\t{uid} {uid} {uid} {uid}\nGid:\t{gid} {gid} {gid} {gid}\nCapInh:\t0\nCapPrm:\t0\nCapEff:\t0\nCapBnd:\t0\nCapAmb:\t0\nNoNewPrivs:\t1\n"
        )
    }
    #[test]
    fn caps_zero_exact_uid_gid_and_nnp_required() {
        let good = status(1000, 1000);
        assert!(validate_status(&good, 1000, 1000).is_ok());
        for bad in [
            good.replace("CapEff:\t0", "CapEff:\t1"),
            good.replace("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
            good.clone() + "Uid:\t1000 1000 1000 1000\n",
        ] {
            assert!(validate_status(&bad, 1000, 1000).is_err());
        }
        assert!(validate_status(&good, 0, 1000).is_err());
        assert!(validate_status(&good, 1001, 1000).is_err());
    }
    #[test]
    fn passwd_mapping_is_exact_not_uid_equals_gid_assumption() {
        assert_eq!(
            parse_primary_gid("user:x:1000:2000::/:/bin/false\n", 1000),
            Ok(2000)
        );
        assert!(parse_primary_gid("user:x:1000:0::/:/bin/false\n", 1000).is_err());
        assert!(
            parse_primary_gid("a:x:1000:1::/:/bin/false\nb:x:1000:2::/:/bin/false\n", 1000)
                .is_err()
        );
    }
}
