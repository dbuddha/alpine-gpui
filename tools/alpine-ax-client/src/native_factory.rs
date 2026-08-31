//! Deterministically testable macOS factory delegation.

use crate::{AxClientError, AxClientFactory, AxGeneration, AxLimits, NativeAxClient, native};

type TrustQuery = fn() -> bool;
type AttachClient = fn(i32, AxGeneration, AxLimits) -> Result<NativeAxClient, AxClientError>;

/// Native generated-binding factory with injectable target-only operations.
#[derive(Clone, Copy, Debug)]
pub struct NativeAxClientFactory {
    trust_query: TrustQuery,
    attach_client: AttachClient,
}

impl Default for NativeAxClientFactory {
    fn default() -> Self {
        Self {
            trust_query: native::is_trusted,
            attach_client: NativeAxClient::attach,
        }
    }
}

impl AxClientFactory for NativeAxClientFactory {
    type Client = NativeAxClient;

    fn is_trusted(&self) -> Result<bool, AxClientError> {
        Ok((self.trust_query)())
    }

    fn attach(
        &self,
        pid: i32,
        generation: AxGeneration,
        limits: AxLimits,
    ) -> Result<Self::Client, AxClientError> {
        (self.attach_client)(pid, generation, limits)
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "fixture setup failures are unrecoverable test harness defects"
)]
mod tests {
    use super::{NativeAxClient, NativeAxClientFactory};
    use crate::{AxClientError, AxClientFactory, AxGeneration, AxLimits};
    use std::time::Duration;

    fn trusted() -> bool {
        true
    }

    fn untrusted() -> bool {
        false
    }

    fn probe_attach(
        pid: i32,
        generation: AxGeneration,
        limits: AxLimits,
    ) -> Result<NativeAxClient, AxClientError> {
        if pid == 42 && generation.get() == 7 && limits.node_limit() == 3 {
            Err(AxClientError::Native {
                operation: "factory-probe",
                code: -7,
            })
        } else {
            Err(AxClientError::InvalidPid)
        }
    }

    fn limits() -> AxLimits {
        AxLimits::new(3, 2, 8, 3, 128, Duration::from_millis(100))
            .unwrap_or_else(|error| panic!("valid native factory limits: {error}"))
    }

    fn generation() -> AxGeneration {
        AxGeneration::new(7)
            .unwrap_or_else(|error| panic!("valid native factory generation: {error}"))
    }

    #[test]
    fn trust_delegation_preserves_both_native_answers() {
        let yes = NativeAxClientFactory {
            trust_query: trusted,
            attach_client: probe_attach,
        };
        let no = NativeAxClientFactory {
            trust_query: untrusted,
            attach_client: probe_attach,
        };
        assert_eq!(yes.is_trusted(), Ok(true));
        assert_eq!(no.is_trusted(), Ok(false));
    }

    #[test]
    fn attach_delegation_preserves_every_admitted_argument() {
        let factory = NativeAxClientFactory {
            trust_query: trusted,
            attach_client: probe_attach,
        };
        assert_eq!(
            factory.attach(42, generation(), limits()).err(),
            Some(AxClientError::Native {
                operation: "factory-probe",
                code: -7,
            })
        );
    }

    #[test]
    fn default_factory_rejects_invalid_pid_before_native_ownership() {
        assert_eq!(
            NativeAxClientFactory::default()
                .attach(0, generation(), limits())
                .err(),
            Some(AxClientError::InvalidPid)
        );
    }
}
