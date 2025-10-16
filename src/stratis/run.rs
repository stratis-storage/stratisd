// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Main loop

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::{
    runtime::Builder,
    select, signal,
    sync::{
        broadcast::channel,
        mpsc::{self, unbounded_channel},
    },
    task,
};

use nix::unistd::getpid;

#[cfg(feature = "dbus_enabled")]
use crate::dbus::DbusAction;
use crate::{
    engine::{
        create_process_keyring, register_clevis_token, set_up_crypt_logging,
        unshare_mount_namespace, Engine, KeyDescription, SimEngine, StratEngine, UdevEngineEvent,
    },
    stratis::{
        dm::dm_event_thread, errors::StratisResult, ipc_support::setup, keys::load_vks,
        stratis::VERSION, timer::run_timers, udev_monitor::udev_thread,
    },
};

// Waits for SIGINT. If received, sends true to all blocking calls in blocking
// threads which will then terminate.
async fn signal_thread() -> StratisResult<()> {
    task::spawn(async {
        if let Err(e) = signal::ctrl_c().await {
            error!("Failure while listening for signals: {e}");
        }
    })
    .await?;

    Ok(())
}

/// Set up all sorts of signal and event handling mechanisms.
/// Initialize the engine and keep it running until a signal is received
/// or a fatal error is encountered.
/// If sim is true, start the sim engine rather than the real engine.
/// Always check for devicemapper context.
pub fn run(sim: bool) -> StratisResult<()> {
    if !sim {
        // If stratisd is running as PID 1, then it is definitely in a container and the
        // unshare_mount_namespace() command method will fail silently, since the mount namespace will
        // remain the same after the unshare command. Also, containers give us a native mount
        // isolation so unsharing the mount namespace is unnecessary. So any filesystem mounts
        // inside a container won't be visible by the host.
        //
        // Note that it is possible for stratisd to be running in a container and not as PID 1, but
        // this situation is less easily detected. However, stratisd will not fail in the
        // unshare_mount_namespace() method if that is the case.
        if getpid().as_raw() != 1 {
            unshare_mount_namespace()?;
        } else {
            debug!("Running as PID 1, not unsharing mount namespace");
        }
    }

    set_up_crypt_logging();
    register_clevis_token()?;
    create_process_keyring()?;

    let runtime = Builder::new_multi_thread()
        .enable_all()
        .thread_name_fn(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            format!("stratis-wt-{id}")
        })
        .on_thread_start(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            trace!("{id}: thread started")
        })
        .on_thread_stop(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            trace!("{id}: thread finished")
        })
        .build()?;
    runtime.block_on(async move {
        async fn start_threads(engine: Arc<dyn Engine>, sim: bool, key_recv: Option<mpsc::UnboundedReceiver<KeyDescription>>) -> StratisResult<()> {
            let (trigger, should_exit) = channel(1);
            let (udev_sender, udev_receiver) = unbounded_channel::<UdevEngineEvent>();
            #[cfg(feature = "dbus_enabled")]
            let (dbus_sender, dbus_receiver) = unbounded_channel::<DbusAction>();

            let join_udev = udev_thread(udev_sender, should_exit);
            let join_ipc = setup(
                Arc::clone(&engine),
                udev_receiver,
                #[cfg(feature = "dbus_enabled")]
                (dbus_sender.clone(), dbus_receiver),
            );
            let join_signal = signal_thread();
            let join_dm = dm_event_thread(
                if sim {
                    None
                } else {
                    Some(Arc::clone(&engine) as Arc<dyn Engine>)
                },
                #[cfg(feature = "dbus_enabled")]
                dbus_sender.clone(),
            );
            let join_timer = run_timers(
                Arc::clone(&engine),
                #[cfg(feature = "dbus_enabled")]
                dbus_sender,
            );
            let vks = load_vks(engine, key_recv);

            select! {
                res = join_udev => {
                    if let Err(e) = res {
                        error!("The udev thread exited with an error: {e}; shutting down stratisd...");
                        return Err(e);
                    } else {
                        info!("The udev thread exited; shutting down stratisd...");
                    }
                }
                res = join_ipc => {
                    if let Err(e) = res {
                        error!("The IPC thread exited with an error: {e}; shutting down stratisd...");
                    } else {
                        info!("The IPC thread exited; shutting down stratisd...");
                    }
                },
                Err(e) = join_dm => {
                    error!("The devicemapper thread exited with an error: {e}; shutting down stratisd...");
                    return Err(e);
                },
                res = join_timer => {
                    if let Err(e) = res {
                        error!("The timer thread exited with an error: {e}; shutting down stratisd...");
                        return Err(e);
                    } else {
                        info!("The timer thread exited; shutting down stratisd...");
                    }
                },
                res = vks => {
                    if let Err(e) = res {
                        error!("The key description thread exited with an error: {e}; shutting down stratisd...");
                        return Err(e);
                    } else {
                        info!("The key description thread exited; shutting down stratisd...");
                    }
                }
                _ = join_signal => {
                    info!("Caught SIGINT; exiting...");
                },
            }

            if let Err(e) = trigger.send(()) {
                warn!("Failed to notify blocking stratisd threads to shut down: {e}");
            }

            Ok(())
        }

        info!("stratis daemon version {VERSION} started");
        if sim {
            info!("Using SimEngine");
            start_threads(Arc::new(SimEngine::default()), sim, None).await
        } else {
            info!("Using StratEngine");
            let (sender, recv) = unbounded_channel();
            start_threads(
                Arc::new(match StratEngine::initialize(sender) {
                    Ok(engine) => engine,
                    Err(e) => {
                        error!("Failed to start up stratisd engine: {e}; exiting");
                        return Err(e);
                    }
                }),
                sim,
                Some(recv),
            ).await
        }
    })?;
    Ok(())
}
