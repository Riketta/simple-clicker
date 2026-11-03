//! Dry-run sink: logs every actuation instead of actuating. Always
//! compiled; selected with `--dry-run` or `sink.backend = "log"`.

use tracing::debug;

use crate::{
    domain::{
        engine::Actuation,
        key::Key,
    },
    ports::driven::{
        InputSink,
        SinkError,
    },
};

/// An [`InputSink`] that records actuations and touches nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoggingSink;

impl InputSink for LoggingSink {
    fn actuate(&mut self, actuation: &Actuation) -> Result<(), SinkError> {
        debug!("dry-run: {actuation}");
        Ok(())
    }

    fn supports(&self, _key: Key) -> bool {
        true
    }

    fn describe(&self) -> String {
        "log (dry-run; nothing is actuated)".to_owned()
    }
}
