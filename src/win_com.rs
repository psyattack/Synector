//! One COM apartment guard for every COM-initialising site.
//!
//! `CoInitializeEx` returns `S_OK` when the call created the apartment on this thread and
//! `S_FALSE` when the thread was already initialised in the same mode. Only the `S_OK` call
//! owns a reference that has to be balanced with `CoUninitialize`; dropping an `S_FALSE`
//! guard would de-initialise the apartment out from under the component that created it.

/// The threading model a COM consumer needs.
#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Apartment {
    MultiThreaded,
    SingleThreaded,
}

#[cfg(windows)]
impl Apartment {
    fn coinit(self) -> windows::Win32::System::Com::COINIT {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED};
        match self {
            Self::MultiThreaded => COINIT_MULTITHREADED,
            Self::SingleThreaded => COINIT_APARTMENTTHREADED,
        }
    }
}

/// Initialises the calling thread and uninitialises it on drop only when this guard owns the
/// initialisation (`S_OK`). `S_FALSE` and a failed call leave `uninit` false, so a nested or
/// rejected initialisation never decrements another component's apartment count.
#[cfg(windows)]
pub(crate) struct ComApartment {
    uninit: bool,
}

#[cfg(windows)]
impl ComApartment {
    /// Infallible entry for callers that treat COM as best-effort.
    pub(crate) fn init(apartment: Apartment) -> Self {
        use windows::Win32::Foundation::S_OK;
        let hr = unsafe { windows::Win32::System::Com::CoInitializeEx(None, apartment.coinit()) };
        Self { uninit: hr == S_OK }
    }

    /// Fallible entry for callers that must report a real COM failure.
    pub(crate) fn try_init(apartment: Apartment) -> Result<Self, String> {
        use windows::Win32::Foundation::S_OK;
        let hr = unsafe { windows::Win32::System::Com::CoInitializeEx(None, apartment.coinit()) };
        hr.ok()
            .map_err(|e| format!("COM initialization failed: {e}"))?;
        Ok(Self { uninit: hr == S_OK })
    }
}

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninit {
            unsafe {
                windows::Win32::System::Com::CoUninitialize();
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn test_repeated_mta_init_does_not_own_the_apartment() {
        // The test harness gives every test its own thread, so the first call initialises
        // COM here (S_OK); the following calls report S_FALSE and must not uninitialise.
        let first = ComApartment::init(Apartment::MultiThreaded);
        assert!(
            first.uninit,
            "the first MTA init on a fresh test thread must own the apartment"
        );

        let second = ComApartment::init(Apartment::MultiThreaded);
        assert!(
            !second.uninit,
            "S_FALSE must not be treated as an owned initialization"
        );

        let third = ComApartment::try_init(Apartment::MultiThreaded)
            .expect("a repeated MTA init must succeed");
        assert!(
            !third.uninit,
            "try_init must apply the same S_OK ownership rule"
        );

        drop(third);
        drop(second);
        assert!(
            first.uninit,
            "nested guards must not take ownership from the first guard"
        );
    }
}
