use std::fmt;

/// Anything that can go wrong while opening a window or setting up the GPU.
#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<E: fmt::Display>(context: &str) -> impl FnOnce(E) -> Error + '_ {
    move |e| Error(format!("{context}: {e}"))
}
