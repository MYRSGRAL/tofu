//! Типизированная граница inline Policy; использует существующий Prepare и его fences.
use crate::Cancellation;
use crate::system_client::{ClientError, Prepared, SystemClient};
use tokio::time::Instant;
use yonik_engine::{Engine, policy_wire::SystemPolicy};
use yonik_import::ServerIntent;

/// Валидированный payload без native config, путей ресурсов и caller claims.
pub struct InlinePolicy(SystemPolicy);
impl InlinePolicy {
    pub fn policy(&self) -> &SystemPolicy {
        &self.0
    }
    pub fn new(policy: SystemPolicy) -> Self {
        Self(policy)
    }
}

impl SystemClient {
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_inline_policy(
        &mut self,
        server: &ServerIntent,
        engine: Engine,
        kill_switch: bool,
        policy: &InlinePolicy,
        deadline: Instant,
        cancel: Cancellation,
    ) -> Result<Prepared, ClientError> {
        self.prepare_profile_policy(server, engine, kill_switch, &policy.0, deadline, cancel)
            .await
    }
}
