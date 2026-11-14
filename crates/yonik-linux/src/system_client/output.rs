//! Неразделимый staged carrier; не вызывает RPC и не разрешает mutation.
use super::Prepared;
use crate::{
    event_log::EventLog,
    output_transport::{self, OutputReaders, ValidatedWriters},
};
use std::io;
use yonik_engine::Engine;

pub(crate) struct PreparedOutput {
    prepared: Prepared,
    readers: OutputReaders,
    writers: ValidatedWriters,
    log: EventLog,
    engine: Engine,
}

// Даже после serialization readers/context не выдаются наружу и не могут rebind.
pub(crate) struct EncodedPreparedOutput {
    prepared: Prepared,
    readers: OutputReaders,
    message: zbus::Message,
    log: EventLog,
    engine: Engine,
}

impl PreparedOutput {
    pub(crate) fn bind(
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
