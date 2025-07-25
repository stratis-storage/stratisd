// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    os::{
        fd::BorrowedFd,
        unix::io::{AsRawFd, RawFd},
    },
    sync::Arc,
};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
#[cfg(feature = "dbus_enabled")]
use tokio::sync::mpsc::UnboundedSender;
use tokio::{io::unix::AsyncFd, task::spawn};

#[cfg(feature = "dbus_enabled")]
use crate::dbus_api::DbusAction;
use crate::{
    engine::{get_dm, get_dm_init, Engine},
    stratis::errors::{StratisError, StratisResult},
};

const REQUIRED_DM_MINOR_VERSION: u32 = 37;

// Waits for devicemapper event. On devicemapper event, transfers control
// to engine to handle event and waits until control is returned from engine.
// Accepts None as an argument; this indicates that devicemapper events are
// to be ignored.
pub async fn dm_event_thread(
    engine: Option<Arc<dyn Engine>>,
    #[cfg(feature = "dbus_enabled")] sender: UnboundedSender<DbusAction>,
) -> StratisResult<()> {
    async fn process_dm_event(
        engine: &Arc<dyn Engine>,
        #[cfg(feature = "dbus_enabled")] sender: &UnboundedSender<DbusAction>,
        fd: &AsyncFd<RawFd>,
    ) -> StratisResult<()> {
        {
            let mut guard = fd.readable().await?;
            // Must clear readiness given that we never actually read any data
            // from the devicemapper file descriptor.
            guard.clear_ready();
        }
        get_dm().arm_poll()?;
        let evented = engine.get_events().await?;

        // NOTE: May need to change order of pool_evented() and fs_evented()

        #[cfg(any(feature = "min", not(feature = "dbus_enabled")))]
        {
            let _ = engine.pool_evented(Some(&evented)).await;
            let _ = engine.fs_evented(Some(&evented)).await;
        }
        #[cfg(feature = "dbus_enabled")]
        {
            let pool_diffs = engine.pool_evented(Some(&evented)).await;
            for action in DbusAction::from_pool_diffs(pool_diffs) {
                if let Err(e) = sender.send(action) {
                    warn!("Failed to update D-Bus layer with changed engine properties: {e}");
                }
            }
            let fs_diffs = engine.fs_evented(Some(&evented)).await;
            for action in DbusAction::from_fs_diffs(fs_diffs) {
                if let Err(e) = sender.send(action) {
                    warn!("Failed to update D-Bus layer with changed engine properties: {e}");
                }
            }
        }

        Ok(())
    }

    spawn(async move {
        match engine {
            Some(engine) => {
                let fd = setup_dm()?;
                loop {
                    trace!("Starting handling of devicemapper event");
                    if let Err(e) = process_dm_event(
                        &engine,
                        #[cfg(feature = "dbus_enabled")]
                        &sender,
                        &fd,
                    )
                    .await
                    {
                        warn!("Failed to process devicemapper event: {e}");
                    }
                    trace!("Finished handling of devicemapper event");
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
    // This version check also implicitly checks for the presence of a working udevd;
    // if udev us not running this function should return an error to prevent
    // stratisd from starting up.
    let minor_dm_version = dm.version()?.1;
    if minor_dm_version < REQUIRED_DM_MINOR_VERSION {
        let err_msg = format!(
            "Requires DM minor version {REQUIRED_DM_MINOR_VERSION} but kernel only supports {minor_dm_version}"
        );
        Err(StratisError::Msg(err_msg))
    } else {
        let fd = get_dm().as_raw_fd();
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
        fcntl(
            borrowed_fd,
            FcntlArg::F_SETFL(
                OFlag::from_bits_truncate(fcntl(borrowed_fd, FcntlArg::F_GETFL)?)
                    & !OFlag::O_NONBLOCK,
            ),
        )?;

        Ok(AsyncFd::new(fd)?)
    }
}
