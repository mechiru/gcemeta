use std::{convert::From, fmt};

/// Represents the details of the [`Error`](struct.Error.html)
#[derive(Debug)]
pub enum ErrorKind {
    /// An error occurred during initialization in the other thread.
    Uninitialized,
    /// Errors that can possibly occur while accessing an HTTP server.
    HttpRequest(attohttpc::Error),
    /// Metadata service response status code other than 200.
    HttpResponse(attohttpc::StatusCode),
    /// Metadata parse error.
    MetadataParse(&'static str),
}

/// Represents errors that can occur during handling metadata service.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
}

impl Error {
    /// Borrow [`ErrorKind`](enum.ErrorKind.html).
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// To own [`ErrorKind`](enum.ErrorKind.html).
    pub fn into_kind(self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ErrorKind::*;
        match self.kind() {
            Uninitialized => write!(
                f,
                "error occurred in a initialization call from the other thread"
            ),
            HttpRequest(e) => write!(f, "http request error: {}", e),
            HttpResponse(code) => write!(f, "http response status code error: {}", code),
            MetadataParse(tag) => write!(f, "metadata parse error: {}", tag),
        }
    }
}

impl ::std::error::Error for Error {}

impl From<attohttpc::Error> for Error {
    fn from(err: attohttpc::Error) -> Self {
        ErrorKind::HttpRequest(err).into()
    }
}

impl From<attohttpc::StatusCode> for Error {
    fn from(code: attohttpc::StatusCode) -> Self {
        ErrorKind::HttpResponse(code).into()
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Error { kind }
    }
}

/// Wrapper for the `Result` type with an [`Error`](struct.Error.html).
pub type Result<T> = ::std::result::Result<T, Error>;
