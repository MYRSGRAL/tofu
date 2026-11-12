//! Opaque pending witness. Не является admission context и не подключён к lookup.
use super::{Process, TrustedContext, bounded, start};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg};
use std::{
    fs,
    io::IoSliceMut,
    mem::MaybeUninit,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    time::{Duration, Instant},
};
const NSFS_MAGIC: i64 = 0x6e736673;
const FRAME: usize = 76;
#[derive(Debug, PartialEq, Eq)]
enum Rejection {
    Sender,
    Identity,
    NamespaceFd,
    Binding,
    Revoked,
    Expired,
    Coverage,
    UnitProofGap,
}
// Ни Clone/Debug/Deserialize, ни constructor из caller-supplied PID/UID/ns inode.
struct PendingWitness {
    worker: Process,
    fds: [OwnedFd; 3],
    cgroup: String,
    binding: [u8; FRAME],
    expires: Instant,
    revoked: bool,
}
fn identity(fd: &OwnedFd) -> Result<(u64, u64), Rejection> {
    if rustix::fs::fstatfs(fd)
        .map_err(|_| Rejection::NamespaceFd)?
        .f_type as i64
        != NSFS_MAGIC
    {
        return Err(Rejection::NamespaceFd);
    }
    let st = rustix::fs::fstat(fd).map_err(|_| Rejection::NamespaceFd)?;
    Ok((st.st_dev, st.st_ino))
}
fn binding(ctx: &TrustedContext) -> [u8; FRAME] {
    let mut b = [0; FRAME];
    b[..4].copy_from_slice(b"TNS1");
    b[4..12].copy_from_slice(&ctx.epoch.to_be_bytes());
    b[12..44].copy_from_slice(&ctx.nonce);
    b[44..76].copy_from_slice(&ctx.digest);
    b
}
impl PendingWitness {
    fn receive(ctx: &TrustedContext, socket: &OwnedFd) -> Result<Self, Rejection> {
        ctx.worker.validate().map_err(|_| Rejection::Identity)?;
        if !rustix::net::sockopt::socket_passcred(socket).map_err(|_| Rejection::Sender)?
            || rustix::net::sockopt::socket_type(socket).map_err(|_| Rejection::Sender)?
                != rustix::net::SocketType::SEQPACKET
        {
            return Err(Rejection::Sender);
        }
        let mut bytes = [0; FRAME + 1];
        let mut space =
            [MaybeUninit::uninit(); rustix::cmsg_space!(ScmCredentials(1), ScmRights(3))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let msg = recvmsg(
            socket,
            &mut [IoSliceMut::new(&mut bytes)],
            &mut ancillary,
            RecvFlags::CMSG_CLOEXEC | RecvFlags::DONTWAIT,
        )
        .map_err(|_| Rejection::Coverage)?;
        let mut credentials = None;
        let mut fds = Vec::with_capacity(3);
        for part in ancillary.drain() {
            match part {
                RecvAncillaryMessage::ScmCredentials(c) if credentials.is_none() => {
                    credentials = Some(c)
                }
                RecvAncillaryMessage::ScmRights(r) => fds.extend(r),
                _ => return Err(Rejection::Sender),
            }
        }
        if msg
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
            || msg.bytes != FRAME
            || fds.len() != 3
        {
            return Err(Rejection::NamespaceFd);
        }
        ctx.worker
            .accept_credentials(credentials.ok_or(Rejection::Sender)?)
            .map_err(|_| Rejection::Sender)?;
        if bytes[..FRAME] != binding(ctx) {
            return Err(Rejection::Binding);
        }
        let fds: [OwnedFd; 3] = fds.try_into().map_err(|_| Rejection::NamespaceFd)?;
        for (fd, expected) in fds.iter().zip(ctx.worker.namespaces) {
            if identity(fd)? != expected {
                return Err(Rejection::NamespaceFd);
            }
        }
        let worker =
            Process::capture(ctx.worker.pid, ctx.worker.uid).map_err(|_| Rejection::Identity)?;
        if worker.start != ctx.worker.start || worker.namespaces != ctx.worker.namespaces {
            return Err(Rejection::Identity);
        }
        let cgroup =
            bounded(&format!("/proc/{}/cgroup", worker.pid)).map_err(|_| Rejection::Coverage)?;
        ctx.worker.validate().map_err(|_| Rejection::Identity)?;
        Ok(Self {
            worker,
            fds,
            cgroup,
            binding: binding(ctx),
            expires: Instant::now() + Duration::from_secs(1),
            revoked: false,
        })
    }
    fn validate_retained(&self, ctx: &TrustedContext) -> Result<(), Rejection> {
        if self.revoked || ctx.revoked {
            return Err(Rejection::Revoked);
        }
        if Instant::now() >= self.expires {
            return Err(Rejection::Expired);
        }
        if self.binding != binding(ctx) {
            return Err(Rejection::Binding);
        }
        let info = bounded(&format!(
            "/proc/self/fdinfo/{}",
            self.worker.pidfd.as_raw_fd()
        ))
        .map_err(|_| Rejection::Identity)?;
        let live = info.lines().find_map(|s| {
            s.strip_prefix("Pid:")
                .and_then(|s| s.trim().parse::<u32>().ok())
        });
        if live != Some(self.worker.pid)
            || start(self.worker.pid).map_err(|_| Rejection::Identity)? != self.worker.start
            || fs::metadata(format!("/proc/{}", self.worker.pid))
                .map_err(|_| Rejection::Identity)?
                .uid()
                != self.worker.uid
        {
            return Err(Rejection::Identity);
        }
        if bounded(&format!("/proc/{}/cgroup", self.worker.pid)).map_err(|_| Rejection::Coverage)?
            != self.cgroup
        {
            return Err(Rejection::Identity);
        }
        for (fd, expected) in self.fds.iter().zip(self.worker.namespaces) {
            if identity(fd)? != expected {
                return Err(Rejection::NamespaceFd);
            }
        }
        Ok(())
    }
    fn admission_gap(&self, ctx: &TrustedContext) -> Result<(), Rejection> {
        self.validate_retained(ctx)?;
        Err(Rejection::UnitProofGap)
    }
    fn revoke(&mut self) {
        self.revoked = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::net::{
        AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketFlags,
        SocketType, sendmsg, socketpair,
    };
    use std::{io::IoSlice, os::fd::AsFd};
    fn context() -> TrustedContext {
        let uid = rustix::process::getuid().as_raw();
        TrustedContext::from_host(std::process::id(), uid, uid, 7, [1; 32], [2; 32]).unwrap()
    }
    fn packet(ctx: &TrustedContext, wrong: bool, swap: bool) -> Result<PendingWitness, Rejection> {
        let (rx, tx) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        rustix::net::sockopt::set_socket_passcred(&rx, true).unwrap();
        let mut files: Vec<fs::File> = ["net", "pid", "mnt"]
            .iter()
            .map(|n| fs::File::open(format!("/proc/self/ns/{n}")).unwrap())
            .collect();
        if wrong {
            files[0] = fs::File::open("/dev/null").unwrap();
        }
        if swap {
            files.swap(0, 1);
        }
        let rights: Vec<_> = files.iter().map(AsFd::as_fd).collect();
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&rights)));
        sendmsg(
            &tx,
            &[IoSlice::new(&binding(ctx))],
            &mut ancillary,
            SendFlags::empty(),
        )
        .unwrap();
        PendingWitness::receive(ctx, &rx)
    }
    #[test]
    fn retained_proof_is_not_unit_immutability_or_admission() {
        let ctx = context();
        let witness = packet(&ctx, false, false).unwrap();
        assert_eq!(witness.validate_retained(&ctx), Ok(()));
        assert_eq!(witness.admission_gap(&ctx), Err(Rejection::UnitProofGap));
    }
    #[test]
    fn malicious_fd_slot_substitution_identity_revoke_and_expiry() {
        let mut ctx = context();
        assert!(matches!(
            packet(&ctx, true, false),
            Err(Rejection::NamespaceFd)
        ));
        assert!(matches!(
            packet(&ctx, false, true),
            Err(Rejection::NamespaceFd)
        ));
        let mut witness = packet(&ctx, false, false).unwrap();
        witness.worker.start ^= 1;
        assert_eq!(witness.validate_retained(&ctx), Err(Rejection::Identity));
        let mut witness = packet(&ctx, false, false).unwrap();
        witness.cgroup.push('x');
        assert_eq!(witness.validate_retained(&ctx), Err(Rejection::Identity));
        let mut witness = packet(&ctx, false, false).unwrap();
        witness.expires = Instant::now();
        assert_eq!(witness.validate_retained(&ctx), Err(Rejection::Expired));
        let mut witness = packet(&ctx, false, false).unwrap();
        witness.revoke();
        assert_eq!(witness.validate_retained(&ctx), Err(Rejection::Revoked));
        let witness = packet(&ctx, false, false).unwrap();
        ctx.epoch += 1;
        assert_eq!(witness.validate_retained(&ctx), Err(Rejection::Binding));
    }
    #[test]
    fn sender_and_pidfd_substitution_are_rejected() {
        let mut child = std::process::Command::new("/usr/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let uid = rustix::process::getuid().as_raw();
        let other = TrustedContext::from_host(child.id(), uid, uid, 7, [1; 32], [2; 32]).unwrap();
        assert!(matches!(
            packet(&other, false, false),
            Err(Rejection::Sender)
        ));
        let ctx = context();
        let mut witness = packet(&ctx, false, false).unwrap();
        witness.worker.pidfd = other.worker.pidfd.try_clone().unwrap();
        assert_eq!(witness.validate_retained(&ctx), Err(Rejection::Identity));
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
