//! "Is the person at the keyboard the owner of this Mac?" — asked before saved
//! credentials are used. On macOS this is LocalAuthentication's device-owner
//! policy: Touch ID (or Apple Watch) with the login password as fallback.

use crate::error::{Error, Result};

pub trait UserPresence: Send + Sync {
    /// Whether this machine can verify its owner at all.
    fn available(&self) -> bool;
    /// Ask the user to authenticate. Blocks until they succeed, fail or cancel;
    /// anything but success is an error. `reason` completes the system
    /// sentence "Orbital is trying to …".
    fn verify(&self, reason: &str) -> Result<()>;
}

/// No way to verify the user (non-macOS builds, headless tests).
pub struct Unavailable;

impl UserPresence for Unavailable {
    fn available(&self) -> bool {
        false
    }
    fn verify(&self, _reason: &str) -> Result<()> {
        Err(Error::PresenceRequired("this device can't verify its owner".into()))
    }
}

#[cfg(target_os = "macos")]
pub use macos::LocalAuth;

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};
    use std::sync::mpsc;

    /// Touch ID / Apple Watch / login password via `LAContext`.
    pub struct LocalAuth;

    // Biometrics with passcode fallback, so Macs without Touch ID still work.
    const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthentication;

    impl UserPresence for LocalAuth {
        fn available(&self) -> bool {
            let ctx = unsafe { LAContext::new() };
            unsafe { ctx.canEvaluatePolicy_error(POLICY) }.is_ok()
        }

        fn verify(&self, reason: &str) -> Result<()> {
            // A fresh context per request: a previous success is never reused.
            let ctx = unsafe { LAContext::new() };
            if let Err(e) = unsafe { ctx.canEvaluatePolicy_error(POLICY) } {
                return Err(Error::PresenceRequired(e.localizedDescription().to_string()));
            }
            let (tx, rx) = mpsc::channel::<std::result::Result<(), String>>();
            let reply = RcBlock::new(move |ok: Bool, err: *mut NSError| {
                let outcome = if ok.as_bool() {
                    Ok(())
                } else {
                    // SAFETY: the system passes either null or a valid NSError.
                    Err(unsafe { err.as_ref() }
                        .map(|e| e.localizedDescription().to_string())
                        .unwrap_or_else(|| "authentication failed".into()))
                };
                let _ = tx.send(outcome);
            });
            unsafe { ctx.evaluatePolicy_localizedReason_reply(POLICY, &NSString::from_str(reason), &reply) };
            // The reply arrives on a private queue; a dropped sender counts as a refusal.
            match rx.recv() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(msg)) => Err(Error::PresenceRequired(msg)),
                Err(_) => Err(Error::PresenceRequired("authentication was interrupted".into())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_never_verifies() {
        assert!(!Unavailable.available());
        assert!(matches!(Unavailable.verify("test"), Err(Error::PresenceRequired(_))));
    }

    /// Shows the real system dialog, so it only runs on request:
    /// `cargo test -p orbital-core presence -- --ignored --nocapture`
    /// Approve it to pass; cancelling must fail the test.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "interactive: shows the Touch ID / password dialog"]
    fn local_auth_prompts_the_user() {
        assert!(LocalAuth.available(), "this Mac has no Touch ID or login password to ask for");
        LocalAuth.verify("run the Orbital presence test").expect("user approved");
    }
}
