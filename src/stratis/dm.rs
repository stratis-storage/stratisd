use std::{
    future::Future,
    os::unix::io::{AsRawFd, RawFd},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::ready;
use tokio::{io::unix::AsyncFd, pin, sync::Mutex};
use tokio_stream::Stream;

use crate::{
    engine::{get_dm, get_dm_init, Engine},
    stratis::errors::{ErrorEnum, StratisError, StratisResult},
};

const REQUIRED_DM_MINOR_VERSION: u32 = 37;

fn setup_dm() -> StratisResult<()> {
    let dm = get_dm_init()?;
    let minor_dm_version = dm.version()?.1;
    if minor_dm_version < REQUIRED_DM_MINOR_VERSION {
        let err_msg = format!(
            "Requires DM minor version {} but kernel only supports {}",
            REQUIRED_DM_MINOR_VERSION, minor_dm_version
        );
        Err(StratisError::Engine(ErrorEnum::Error, err_msg))
    } else {
        Ok(())
    }
}

pub struct DmFd {
    engine: Arc<Mutex<dyn Engine>>,
    fd: AsyncFd<RawFd>,
}

impl DmFd {
    /// Constructs a DmFd struct containing a reference to the engine and a DM
    /// context file descriptor.
    pub fn new(engine: Arc<Mutex<dyn Engine>>) -> StratisResult<DmFd> {
        setup_dm()?;

        Ok(DmFd {
            engine,
            fd: AsyncFd::new(get_dm().as_raw_fd())?,
        })
    }
}

impl Stream for DmFd {
    type Item = StratisResult<()>;

    /// When called, waits until DM file descriptor is ready, then locks the
    /// engine, and rearms the event mechanism.
    /// Then causes the engine to handle the DM event.
    /// Never returns None, as there can always be a next DM event.
    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context) -> Poll<Option<StratisResult<()>>> {
        let mut ready_guard = ready!(self.fd.poll_read_ready(cxt))?;
        ready_guard.clear_ready();
        let lock_future = self.engine.lock();
        pin!(lock_future);
        let mut lock = ready!(lock_future.poll(cxt));
        get_dm().arm_poll()?;
        lock.evented()?;
        Poll::Ready(Some(Ok(())))
    }
}
