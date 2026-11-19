//! Test-only process termination at actual storage commit edges. Product
//! binaries neither read these environment variables nor terminate here.
use std::io;
#[cfg(test)]
thread_local! {
    static ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAILURE: std::cell::Cell<Option<(&'static str, io::ErrorKind)>> = const { std::cell::Cell::new(None) };
}
#[cfg(test)]
pub(super) fn fail_once(point: &'static str, kind: io::ErrorKind) {
    FAILURE.set(Some((point, kind)));
}
#[cfg(test)]
pub(super) fn arm() {
    ARMED.set(true);
}
#[inline]
pub(super) fn edge(_point: &str) -> io::Result<()> {
    #[cfg(test)]
    if let Some((point, kind)) = FAILURE.get()
        && point == _point
    {
        FAILURE.set(None);
        return Err(io::Error::new(kind, "injected storage commit failure"));
    }
    #[cfg(test)]
    if ARMED.get() && std::env::var("HEX_RECOVERY_TEST_EDGE").as_deref() == Ok(_point) {
        std::process::exit(86);
    }
    Ok(())
}
