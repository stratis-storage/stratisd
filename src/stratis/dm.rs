use std::{
    future::Future,
    os::unix::io::AsRawFd,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::ready;
use tokio::{io::unix::AsyncFd, pin, sync::Mutex};
use tokio_stream::Stream;

use crate::{async_fd::FdWrapper, engine::Engine, stratis::errors::StratisResult};

pub struct DmFd {
    engine: Arc<Mutex<dyn Engine>>,
    fd: AsyncFd<FdWrapper>,
}

impl DmFd {
    pub async fn new(engine: Arc<Mutex<dyn Engine>>) -> StratisResult<Option<DmFd>> {
        let fd = {
            let lock = engine.lock().await;
            if let Some(evt) = lock.get_dm_context() {
                evt.as_raw_fd()
            } else {
                return Ok(None);
            }
        };
        Ok(Some(DmFd {
            engine,
            fd: AsyncFd::new(FdWrapper::new(fd))?,
        }))
    }
}

impl Stream for DmFd {
    type Item = StratisResult<()>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context) -> Poll<Option<StratisResult<()>>> {
        let mut ready_guard = ready!(self.fd.poll_read_ready(cxt))?;
        ready_guard.clear_ready();
        let lock_future = self.engine.lock();
        pin!(lock_future);
        let mut lock = ready!(lock_future.poll(cxt));
        if let Some(evt) = lock.get_dm_context() {
            evt.arm_poll()?;
            lock.evented()?;
            Poll::Ready(Some(Ok(())))
        } else {
            Poll::Ready(None)
        }
    }
}
