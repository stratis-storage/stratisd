use std::os::unix::io::{AsRawFd, RawFd};

pub struct FdWrapper(RawFd);

impl FdWrapper {
    pub fn new(fd: RawFd) -> FdWrapper {
        FdWrapper(fd)
    }
}

impl AsRawFd for FdWrapper {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}
