use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);

#[derive(Debug, thiserror::Error)]
#[error("Browser import cancelled")]
pub(crate) struct Cancelled;

impl Cancellation {
    #[cfg(any(feature = "native-gui", test))]
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn check(&self) -> anyhow::Result<()> {
        if self.is_cancelled() {
            Err(Cancelled.into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_shared_and_keeps_its_type_through_context() {
        let cancellation = Cancellation::default();
        cancellation.check().unwrap();
        cancellation.clone().cancel();
        assert!(cancellation.is_cancelled());
        let error = cancellation.check().unwrap_err().context("import failed");
        assert!(error.is::<Cancelled>());
    }
}
