//! End-to-end use of the retention envelope across a simulated boot sequence.
//!
//! A `RetainedCell` static stands in for the RTC-placed cell, and a mock
//! `ResetReasonSource` plays the platform tier, so the test exercises the
//! public API exactly the way firmware does: read the reset reason, load, fall
//! back to fresh state, bump, store, "reset", repeat.

use stoker::{
    Invalid, NoopResetReasonSource, ResetReason, ResetReasonSource, Retainable, RetainedCell,
};

/// Payload a consumer would retain: a session id fixed at cold boot and a
/// counter bumped on every boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Session {
    id: u32,
    boots: u32,
}

impl Retainable for Session {
    const VERSION: u16 = 1;
    const SIZE: usize = 8;

    fn write(&self, buf: &mut [u8]) -> Option<()> {
        buf.get_mut(0..4)?.copy_from_slice(&self.id.to_le_bytes());
        buf.get_mut(4..8)?
            .copy_from_slice(&self.boots.to_le_bytes());
        Some(())
    }

    fn read(buf: &[u8]) -> Option<Self> {
        let id = u32::from_le_bytes(buf.get(0..4)?.try_into().ok()?);
        let boots = u32::from_le_bytes(buf.get(4..8)?.try_into().ok()?);
        Some(Self { id, boots })
    }
}

/// The RTC-placed cell; in firmware the tier adds its section attribute.
static RETAINED: RetainedCell<16> = RetainedCell::new();

/// One firmware boot: load or start fresh, bump, store, return what ran.
fn boot(resets: &impl ResetReasonSource, fresh_id: u32) -> (Result<Session, Invalid>, Session) {
    let loaded = RETAINED.load::<Session>(resets.last_reset_reason());
    let mut session = loaded.unwrap_or(Session {
        id: fresh_id,
        boots: 0,
    });
    session.boots += 1;
    RETAINED.store(&session).unwrap();
    (loaded, session)
}

#[test]
fn session_survives_sleep_and_resets_but_not_power_loss() {
    // Cold boot: nothing to load, a fresh session starts.
    let (loaded, s1) = boot(&NoopResetReasonSource::default(), 0xA11C_E000);
    assert_eq!(loaded, Err(Invalid::PowerOn));
    assert_eq!(
        s1,
        Session {
            id: 0xA11C_E000,
            boots: 1
        }
    );

    // Ten deep-sleep cycles keep the session id and count every wake.
    let wake = NoopResetReasonSource::with_reason(ResetReason::DeepSleep);
    let mut last = s1;
    for _ in 0..10 {
        let (loaded, next) = boot(&wake, 0xBAD);
        assert_eq!(loaded, Ok(last));
        assert_eq!(next.id, s1.id);
        assert_eq!(next.boots, last.boots + 1);
        last = next;
    }
    assert_eq!(last.boots, 11);

    // Watchdog, software and panic resets trust the checksum and keep the id.
    for reason in [
        ResetReason::Watchdog,
        ResetReason::Software,
        ResetReason::Panic,
        ResetReason::Other,
    ] {
        let (loaded, next) = boot(&NoopResetReasonSource::with_reason(reason), 0xBAD);
        assert_eq!(loaded, Ok(last), "{reason:?}");
        assert_eq!(next.id, s1.id);
        last = next;
    }
    assert_eq!(last.boots, 15);

    // A brownout discards the record even though its bits are intact.
    let (loaded, s2) = boot(
        &NoopResetReasonSource::with_reason(ResetReason::Brownout),
        0xB0B0_0001,
    );
    assert_eq!(loaded, Err(Invalid::Brownout));
    assert_eq!(
        s2,
        Session {
            id: 0xB0B0_0001,
            boots: 1
        }
    );

    // The next wake continues from the post-brownout session.
    let (loaded, s3) = boot(&wake, 0xBAD);
    assert_eq!(loaded, Ok(s2));
    assert_eq!(
        s3,
        Session {
            id: 0xB0B0_0001,
            boots: 2
        }
    );
}
