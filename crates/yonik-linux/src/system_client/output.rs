//! Одноразовый legacy Apply output: Prepared, две пары и immutable journal scope.
use super::Prepared;
use crate::{
    event_log::EventLog,
    output_transport::{self, OutputReaders, ValidatedWriters},
};
use std::io;
use yonik_engine::Engine;

pub struct PreparedOutput {
    prepared: Prepared,
    readers: OutputReaders,
    writers: ValidatedWriters,
    log: EventLog,
    engine: Engine,
}

// Даже после serialization readers/context не выдаются наружу и не могут rebind.
#[cfg(test)]
pub(crate) struct EncodedPreparedOutput {
    prepared: Prepared,
    readers: OutputReaders,
    message: zbus::Message,
    log: EventLog,
    engine: Engine,
}

impl PreparedOutput {
    pub fn bind(
        prepared: Prepared,
        engine: Engine,
        root_log: &EventLog,
        generation: u64,
    ) -> io::Result<Self> {
        if generation == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "нулевое поколение output",
            ));
        }
        let log = root_log
            .session_scope()
            .ok_or_else(|| io::Error::other("исчерпан session id"))?
            .with_generation(generation);
        let (readers, writers) = output_transport::create()?;
        Ok(Self {
            prepared,
            readers,
            writers: writers.validate()?,
            log,
            engine,
        })
    }

    // Consume не оставляет Prepared или sender writers вызывающему коду для retry.
    #[cfg(test)]
    pub(crate) fn encode(self) -> io::Result<EncodedPreparedOutput> {
        let message = output_transport::encode(self.prepared.handle, self.writers)?;
        Ok(EncodedPreparedOutput {
            prepared: self.prepared,
            readers: self.readers,
            message,
            log: self.log,
            engine: self.engine,
        })
    }
}

#[cfg(test)]
mod tests;

/// Readers принадлежат конкретному Apply; EOF не является доказательством stop.
pub struct OutputDrain(Vec<tokio::task::JoinHandle<()>>);
impl OutputDrain {
    pub async fn drained(self) -> bool {
        let mut complete = true;
        for task in self.0 {
            complete &= task.await.is_ok();
        }
        complete
    }
}
fn reader(
    mut pipe: tokio::net::unix::pipe::Receiver,
    engine: Engine,
    log: EventLog,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut input = crate::event_log::native_details::Input::new(engine, log.clone());
        let mut bytes = zeroize::Zeroizing::new([0u8; 16384]);
        loop {
            let n = match pipe.read(&mut bytes[..]).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => {
                    log.reject();
                    break;
                }
            };
            let mut offset = 0;
            while offset < n {
                offset += input.feed(&bytes[offset..n]);
                tokio::task::yield_now().await;
            }
            bytes[..n].fill(0);
        }
    })
}
impl super::SystemClient {
    pub async fn apply_output(
        &mut self,
        output: PreparedOutput,
        deadline: tokio::time::Instant,
        cancel: crate::Cancellation,
    ) -> (
        Result<super::State, super::ClientError>,
        Option<OutputDrain>,
    ) {
        let PreparedOutput {
            prepared,
            readers,
            writers,
            log,
            engine,
        } = output;
        if prepared.owner != self.owner
            || Some(prepared.peer.as_str()) != self.connection.unique_name().map(|n| n.as_str())
        {
            return (Err(super::ClientError::Stale), None);
        }
        let drain = OutputDrain(vec![
            reader(readers.stdout, engine, log.clone()),
            reader(readers.stderr, engine, log),
        ]);
        let files = [
            std::fs::File::from(writers.stdout),
            std::fs::File::from(writers.stderr),
        ];
        let request = serde_json::json!({"version":1,"handle":prepared.handle}).to_string();
        let result = async {
            let json = self
                .call_with_resources("ApplyOutputV1", request, &files, deadline, cancel)
                .await?;
            let reply: super::StateReply = super::decode(&json)?;
            super::version(reply.version)?;
            Ok(reply.state)
        }
        .await;
        (result, Some(drain))
    }
}
