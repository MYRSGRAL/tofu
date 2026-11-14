//! Consuming admission: installed producer, authenticated report и owning custody.
#![allow(dead_code)]
use super::{
    kernel_evidence::{self, Expected}, native_profile::VerifiedInstalledProfile,
    run_authorization::{RunAuthority, RunAuthorization, RunRevocationGuard},
    BootstrapUnitObservation, SystemWorkerSupervisor, WorkerSupervisorError as Error,
};
use crate::{Cancellation, worker::anchor_exchange::{Challenge, NamespaceObservation}};
use std::{os::unix::net::UnixStream, time::Instant};
use tokio::time::Instant as Deadline;

type Result<T> = std::result::Result<T, Error>;

pub(crate) struct VerifiedPendingRun {
    profile: VerifiedInstalledProfile,
    authorization: RunAuthorization,
    expected: Expected,
    deadline: Deadline,
    guard: RunRevocationGuard,
}

pub(crate) struct VerifiedCoverage {
    profile: VerifiedInstalledProfile,
    authorization: RunAuthorization,
    expected: Expected,
    custody: NamespaceObservation,
    guard: RunRevocationGuard,
    deadline: Deadline,
}

impl VerifiedPendingRun {
    pub(crate) async fn verify(
        supervisor: &SystemWorkerSupervisor,
        owner: &RunAuthority,
        authorization: RunAuthorization,
        observation: BootstrapUnitObservation,
        digest: [u8;32], epoch: u64,
        cancel: &mut Cancellation, deadline: Deadline,
    ) -> Result<Self> {
        authorization.validate_owner(owner).map_err(|_| Error::Untrusted)?;
        let guard = authorization.guard();
        if Deadline::now() >= deadline || epoch == 0 || digest == [0;32] {
            return Err(Error::Stale);
        }
        let profile = VerifiedInstalledProfile::verify(supervisor, observation, cancel, deadline).await?;
        if profile.policy_digest() != yonik_nsfs::fixed_self_fence::PROFILE_DIGEST {
            return Err(Error::Untrusted);
        }
        let (nonce, session, generation) = authorization.challenge_context().map_err(|_| Error::Untrusted)?;
        let observation = profile.observation();
        let mut observations = [1;19]; observations[2] = 38;
        let expected = Expected {
            nonce, digest, session, generation, epoch,
            invocation: observation.native_invocation(),
            task_pid: observation.native_pid(), task_start: observation.native_start(),
            server_pid: std::process::id(), server_start: own_start()?,
            profile: profile.policy_digest(), observations,
        };
        authorization.validate_owner(owner).map_err(|_| Error::Untrusted)?;
        if Deadline::now() >= deadline { return Err(Error::Stale); }
        Ok(Self { profile, authorization, expected, deadline, guard })
    }
    pub(crate) async fn receive(
        self, supervisor: &SystemWorkerSupervisor, owner: &RunAuthority,
        stream: &UnixStream, cancel: &mut Cancellation,
    ) -> Result<VerifiedCoverage> {
        self.authorization.validate_owner(owner).map_err(|_| Error::Untrusted)?;
        if Deadline::now() >= self.deadline { return Err(Error::Stale); }
        let challenge = Challenge::for_authorization(&self.authorization, self.expected.digest, self.expected.invocation)
            .map_err(|_| Error::Protocol)?;
        let (custody, frame) = challenge.exchange_v4_bound(
            stream, self.expected.task_pid, self.profile.observation().native_uid(),
            self.deadline.into_std(), self.expected.epoch,
        ).map_err(|_| Error::Protocol)?;
        let profile = self.profile.revalidate(supervisor, cancel, self.deadline).await?;
        self.authorization.validate_owner(owner).map_err(|_| Error::Untrusted)?;
        if Deadline::now() >= self.deadline { return Err(Error::Stale); }
        kernel_evidence::verify_fixed_report(&frame, &self.expected).map_err(|_| Error::UnitProofGap)?;
        self.authorization.bind_custody(&custody).map_err(|_| Error::UnitProofGap)?;
        Ok(VerifiedCoverage { profile, authorization: self.authorization, expected: self.expected,
            custody, guard: self.guard, deadline: self.deadline })
    }
}

fn own_start() -> Result<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").map_err(|_| Error::Unavailable)?;
    stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(19))
        .and_then(|value| value.parse().ok()).filter(|value| *value != 0).ok_or(Error::Untrusted)
}
