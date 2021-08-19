// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::{AsRawFd, RawFd};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use tokio::{io::unix::AsyncFd, task::spawn};

use crate::{
    engine::{get_dm, get_dm_init, Engine, LockableEngine},
    stratis::errors::{StratisError, StratisResult},
};

const REQUIRED_DM_MINOR_VERSION: u32 = 37;

// Waits for devicemapper event. On devicemapper event, transfers control
// to engine to handle event and waits until control is returned from engine.
// Accepts None as an argument; this indicates that devicemapper events are
// to be ignored.
pub async fn dm_event_thread<E>(engine: Option<LockableEngine<E>>) -> StratisResult<()>
where
    E: 'static + Engine,
{
    async fn process_dm_event<E>(
        engine: &LockableEngine<E>,
        fd: &AsyncFd<RawFd>,
    ) -> StratisResult<()>
    where
        E: Engine,
    {
        {
            let mut guard = fd.readable().await?;
            guard.clear_ready();
        }
        get_dm().arm_poll()?;
        let mut lock = engine.lock().await;
        lock.evented()?;
        Ok(())
    }

    spawn(async {
        match engine {
            Some(engine) => {
                let fd = setup_dm()?;
                loop {
                    if let Err(e) = process_dm_event(&engine, &fd).await {
                        warn!("Failed to process devicemapper event: {}", e);
                    }
                }
            }
            None => {
                info!("devicemapper event monitoring disabled in sim engine");
                Result::<_, StratisError>::Ok(())
            }
        }
    })
    .await??;

    Ok(())
}

/// Set the devicemapper file descriptor to nonblocking and create an asynchronous
/// context for polling for events.
fn setup_dm() -> StratisResult<AsyncFd<RawFd>> {
    let dm = get_dm_init()?;
    let minor_dm_version = dm.version()?.1;
    if minor_dm_version < REQUIRED_DM_MINOR_VERSION {
        let err_msg = format!(
            "Requires DM minor version {} but kernel only supports {}",
            REQUIRED_DM_MINOR_VERSION, minor_dm_version
        );
        Err(StratisError::Msg(err_msg))
    } else {
        let fd = get_dm().as_raw_fd();
        fcntl(
            fd,
            FcntlArg::F_SETFL(
                OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?) & !OFlag::O_NONBLOCK,
            ),
        )?;

        Ok(AsyncFd::new(fd)?)
    }
}
