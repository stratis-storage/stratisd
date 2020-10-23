use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::ready;
use tokio::{io::unix::AsyncFd, pin, stream::Stream, sync::Mutex};

use crate::{async_fd::FdWrapper, engine::Engine, stratis::errors::StratisResult};

pub struct DmFd {
    engine: Arc<Mutex<dyn Engine>>,
    fd: AsyncFd<FdWrapper>,
}

impl DmFd {
    pub async fn new(engine: Arc<Mutex<dyn Engine>>) -> StratisResult<Option<DmFd>> {
        let fd = {
            let lock = engine.lock().await;
            if let Some(evt) = (*lock).get_eventable() {
                evt.get_pollable_fd()
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
        let _ = ready!(self.fd.poll_read_ready(cxt));
        let lock_future = self.engine.lock();
        pin!(lock_future);
        let mut lock = ready!(lock_future.poll(cxt));
        if let Some(evt) = (*lock).get_eventable() {
            evt.clear_event()?;
            (*lock).evented()?;
            Poll::Ready(Some(Ok(())))
        } else {
            Poll::Ready(None)
        }
    }
}
