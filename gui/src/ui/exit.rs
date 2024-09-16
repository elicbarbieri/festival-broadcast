//---------------------------------------------------------------------------------------------------- Use
//use anyhow::{anyhow,bail,ensure};
//use log::{info,error,warn,trace,debug};
//use serde::{Serialize,Deserialize};
//use crate::macros::*;
//use disk::prelude::*;
//use disk::{};
//use std::{};
use crate::constants::{SETTINGS_VERSION, STATE_VERSION};
use crate::data::Gui;
use crate::data::{Settings, State, EXIT_COUNTDOWN, SHOULD_EXIT};
use benri::{log::*, sync::*, thread::*};
use crossbeam::channel::{Receiver, Sender};
use disk::{Bincode2, Json, Toml};
use log::{debug, error, info};
use shukusai::kernel::{FrontendToKernel, KernelToFrontend};
use std::time::{Duration, Instant};

//---------------------------------------------------------------------------------------------------- Gui::exit() - The thread that handles exiting.
impl Gui {
    pub(super) fn spawn_exit_thread(&mut self) {
        // INVARIANT: This function should only be called once,
        // and the Gui.exiting should be set to true.

        // Clone things to send to exit thread.
        let to_kernel = self.to_kernel.clone();
        let from_kernel = self.from_kernel.clone();
        let settings = self.settings.clone();
        let state = self.state.clone();

        // Spawn `exit` thread.
        std::thread::spawn(move || Self::exit(to_kernel, from_kernel, state, settings));

        // Set the exit `Instant`.
        self.exit_instant = Instant::now();
    }

    #[inline(always)]
    pub(super) fn exit(
        to_kernel: Sender<FrontendToKernel>,
        from_kernel: Receiver<KernelToFrontend>,
        state: State,
        settings: Settings,
    ) {
        // Tell `Kernel` to save stuff.
        send!(to_kernel, FrontendToKernel::Exit);

        // Save `State`.
        match state.save() {
            Ok(md) => ok!("GUI - State{STATE_VERSION} save: {md}"),
            Err(e) => fail!("GUI - State{STATE_VERSION} save: {e}"),
        }

        // Save `Settings`.
        match settings.save_atomic() {
            Ok(md) => ok!("GUI - Settings{SETTINGS_VERSION} save: {md}"),
            Err(e) => fail!("GUI - Settings{SETTINGS_VERSION} save: {e}"),
        }

        // Check if `Kernel` succeeded.
        // Loop through 3 messages just in-case
        // there were others in the channel queue.
        //
        // This waits a max `900ms` before
        // continuing without the response.
        let mut n = 0;
        loop {
            if let Ok(KernelToFrontend::Exit(r)) =
                from_kernel.recv_timeout(Duration::from_millis(300))
            {
                match r {
                    Ok(_) => debug!("GUI - Kernel save"),
                    Err(e) => debug!("GUI - Kernel save failed: {e}"),
                }
                break;
            } else if n > 3 {
                debug!("GUI - Could not determine Kernel's exit result");
            } else {
                n += 1;
            }
        }

        // Wait until `Collection` is saved,
        // or until we've elapsed total time.
        loop {
            let e = atomic_load!(EXIT_COUNTDOWN);

            if e == 0 {
                // Exit with error.
                error!("GUI - Collection save is taking more than {e} seconds, skipping save...!");
                break;
            }

            if shukusai::state::saving() {
                atomic_sub!(EXIT_COUNTDOWN, 1);
                info!("GUI - Waiting for Collection to be saved, force exit in [{e}] seconds");
                sleep!(1);
            } else {
                break;
            }
        }

        std::process::exit(0);
    }
}

//---------------------------------------------------------------------------------------------------- TESTS
//#[cfg(test)]
//mod tests {
//  #[test]
//  fn __TEST__() {
//  }
//}
