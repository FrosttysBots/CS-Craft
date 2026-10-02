//! Scheduling priority for engine threads: background workers yield to the
//! render and server threads, so a busy pool cannot preempt a frame.
//! Only Windows is handled; elsewhere these do nothing.

#[cfg(windows)]
mod ffi {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetCurrentThread() -> *mut c_void;
        pub fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
        pub fn GetCurrentProcess() -> *mut c_void;
        pub fn SetProcessInformation(process: *mut c_void, class: i32, information: *const c_void, size: u32) -> i32;
        pub fn SetPriorityClass(process: *mut c_void, class: u32) -> i32;
    }

    /// `PROCESS_POWER_THROTTLING_STATE`.
    #[repr(C)]
    pub struct PowerThrottlingState {
        pub version: u32,
        pub control_mask: u32,
        pub state_mask: u32,
    }
}

#[cfg(windows)]
fn set(priority: i32) {
    // SAFETY: the pseudo handle of the calling thread is always valid.
    unsafe {
        ffi::SetThreadPriority(ffi::GetCurrentThread(), priority);
    }
}

#[cfg(not(windows))]
fn set(_priority: i32) {}

/// `THREAD_PRIORITY_BELOW_NORMAL`, for worker pools.
pub fn background() {
    set(-1);
}

/// Asks Windows not to power-throttle this process (EcoQoS): without it a
/// process in the background, such as a benchmark run from a console, can
/// have its threads held to low clock speeds.
pub fn no_power_throttling() {
    #[cfg(windows)]
    {
        // ProcessPowerThrottling, controlling EXECUTION_SPEED, turned off.
        let state = ffi::PowerThrottlingState { version: 1, control_mask: 1, state_mask: 0 };
        // SAFETY: the pseudo handle of this process and a correctly sized state.
        unsafe {
            ffi::SetProcessInformation(ffi::GetCurrentProcess(), 4, &state as *const _ as *const std::ffi::c_void, std::mem::size_of::<ffi::PowerThrottlingState>() as u32);
        }
    }
}

/// `IDLE_PRIORITY_CLASS` for the whole process: benchmarks and tools that
/// must not take CPU time from anything else running.
pub fn idle_process() {
    #[cfg(windows)]
    {
        // SAFETY: the pseudo handle of this process.
        unsafe {
            ffi::SetPriorityClass(ffi::GetCurrentProcess(), 0x40);
        }
    }
}

/// `THREAD_PRIORITY_ABOVE_NORMAL`, for the render thread.
pub fn interactive() {
    set(1);
}
