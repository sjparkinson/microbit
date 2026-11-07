//! The micro:bit's CMSIS-DAP HID interface: find it, open it, and swap
//! 64-byte reports with it. IOKit on macOS, hidraw on Linux, and no crates,
//! so this is the one file in xtask with `unsafe` (the IOKit calls).
//!
//! Every micro:bit's interface chip enumerates as Arm's DAPLink, USB ID
//! 0d28:0204, with the board's unique ID as its serial number.

#![allow(unsafe_code)]

pub const VENDOR: u16 = 0x0D28;
pub const PRODUCT: u16 = 0x0204;
/// DAPLink's HID reports are 64 bytes, with no report ID.
const REPORT: usize = 64;
/// How long to wait for the probe to answer one command.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// A micro:bit on USB, by its serial number, which starts with the board ID.
#[derive(Clone)]
pub struct Probe {
    pub serial: String,
    #[cfg(target_os = "linux")]
    path: std::path::PathBuf,
}

/// Pads a command out to a whole report.
fn report(command: &[u8]) -> Result<[u8; REPORT], String> {
    let mut report = [0u8; REPORT];
    report
        .get_mut(..command.len())
        .ok_or("a command bigger than a HID report")?
        .copy_from_slice(command);
    Ok(report)
}

#[cfg(target_os = "macos")]
pub use macos::{Hid, list};

#[cfg(target_os = "linux")]
pub use linux::{Hid, list};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub use other::{Hid, list};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod other {
    //! Anywhere else: no USB, so no flashing.

    use super::Probe;

    const UNSUPPORTED: &str = "flashing over USB only works on macOS and Linux";

    pub fn list() -> Result<Vec<Probe>, String> {
        Err(UNSUPPORTED.into())
    }

    /// Can't be made, since `list` never finds anything to open.
    pub enum Hid {}

    impl Hid {
        pub fn open(_: &Probe) -> Result<Self, String> {
            Err(UNSUPPORTED.into())
        }
    }

    impl crate::dap::Link for Hid {
        fn exchange(&mut self, _: &[u8]) -> Result<Vec<u8>, String> {
            match *self {}
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    //! hidraw: each /sys/class/hidraw/hidrawN/device/uevent names the USB ID
    //! and serial, and /dev/hidrawN takes and gives raw reports.

    use std::ffi::{c_int, c_short, c_ulong};
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::{PRODUCT, Probe, REPORT, TIMEOUT, VENDOR, report};

    /// poll(2)'s `struct pollfd`, from <poll.h>.
    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: c_short,
        revents: c_short,
    }

    const POLLIN: c_short = 0x001;

    unsafe extern "C" {
        fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
    }

    /// Waits up to `timeout` for `file` to have something to read, and says
    /// whether it does.
    fn readable(file: &fs::File, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let ms = c_int::try_from(left.as_millis()).unwrap_or(c_int::MAX);
            let mut fd = PollFd {
                fd: file.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd, for an fd `file` keeps open.
            let n = unsafe { poll(&raw mut fd, 1, ms) };
            match n {
                0 => return Ok(false),
                1.. => return Ok(true),
                _ => {
                    let e = io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::Interrupted {
                        return Err(e);
                    }
                }
            }
        }
    }

    pub fn list() -> Result<Vec<Probe>, String> {
        let id = format!("HID_ID=0003:{:08X}:{:08X}", VENDOR, PRODUCT);
        let mut probes = Vec::new();
        let Ok(entries) = fs::read_dir("/sys/class/hidraw") else {
            return Ok(probes);
        };
        for entry in entries.flatten() {
            let uevent = fs::read_to_string(entry.path().join("device/uevent")).unwrap_or_default();
            if !uevent.lines().any(|line| line == id) {
                continue;
            }
            let serial = uevent
                .lines()
                .find_map(|line| line.strip_prefix("HID_UNIQ="))
                .unwrap_or_default()
                .to_string();
            probes.push(Probe {
                serial,
                path: PathBuf::from("/dev").join(entry.file_name()),
            });
        }
        probes.sort_by(|a, b| a.serial.cmp(&b.serial));
        Ok(probes)
    }

    pub struct Hid(fs::File);

    impl Hid {
        pub fn open(probe: &Probe) -> Result<Self, String> {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&probe.path)
                .map(Hid)
                .map_err(|e| {
                    format!(
                        "{}: {e}; on Linux that usually needs a udev rule for 0d28:0204",
                        probe.path.display()
                    )
                })
        }
    }

    impl crate::dap::Link for Hid {
        fn exchange(&mut self, command: &[u8]) -> Result<Vec<u8>, String> {
            // A reply that turned up after its command timed out would
            // otherwise answer this one.
            let mut stale = [0u8; REPORT];
            while readable(&self.0, Duration::ZERO).unwrap_or(false) {
                if !matches!(self.0.read(&mut stale), Ok(1..)) {
                    break;
                }
            }
            // hidraw wants the report ID first, which is 0 for none.
            let mut out = vec![0u8];
            out.extend(report(command)?);
            self.0
                .write_all(&out)
                .map_err(|e| format!("writing to the probe: {e}"))?;
            let ready =
                readable(&self.0, TIMEOUT).map_err(|e| format!("waiting for the probe: {e}"))?;
            if !ready {
                return Err(format!(
                    "the micro:bit didn't answer within {} s",
                    TIMEOUT.as_secs()
                ));
            }
            let mut response = vec![0u8; REPORT];
            let n = self
                .0
                .read(&mut response)
                .map_err(|e| format!("reading from the probe: {e}"))?;
            response.truncate(n);
            Ok(response)
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! IOKit's HID manager, through the C API. Output reports go with
    //! IOHIDDeviceSetReport; input reports arrive on a callback, which runs
    //! when this thread's run loop does.

    use std::collections::VecDeque;
    use std::ffi::{CStr, c_char, c_void};
    use std::time::{Duration, Instant};

    use super::{PRODUCT, Probe, REPORT, TIMEOUT, VENDOR, report};

    type CFTypeRef = *const c_void;
    type CFIndex = isize;
    type IOReturn = i32;

    const K_CF_NUMBER_SINT32: CFIndex = 3;
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_IOHID_REPORT_TYPE_OUTPUT: u32 = 1;
    const K_IORETURN_SUCCESS: IOReturn = 0;
    /// CFRunLoopRunInMode's result when it handled a source.
    const K_CF_RUN_LOOP_RUN_HANDLED_SOURCE: i32 = 4;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        static kCFRunLoopDefaultMode: CFTypeRef;
        fn CFDictionaryCreateMutable(
            allocator: CFTypeRef,
            capacity: CFIndex,
            keys: *const c_void,
            values: *const c_void,
        ) -> CFTypeRef;
        fn CFDictionarySetValue(dict: CFTypeRef, key: CFTypeRef, value: CFTypeRef);
        fn CFNumberCreate(allocator: CFTypeRef, kind: CFIndex, value: *const c_void) -> CFTypeRef;
        fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            s: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFStringGetCString(
            s: CFTypeRef,
            buffer: *mut c_char,
            size: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFStringGetTypeID() -> usize;
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFSetGetCount(set: CFTypeRef) -> CFIndex;
        fn CFSetGetValues(set: CFTypeRef, values: *mut CFTypeRef);
        fn CFRelease(cf: CFTypeRef);
        fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
        fn CFRunLoopGetCurrent() -> CFTypeRef;
        fn CFRunLoopRunInMode(mode: CFTypeRef, seconds: f64, return_after_source: u8) -> i32;
    }

    type ReportCallback = extern "C" fn(
        context: *mut c_void,
        result: IOReturn,
        sender: *mut c_void,
        kind: u32,
        id: u32,
        report: *mut u8,
        length: CFIndex,
    );

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOHIDManagerCreate(allocator: CFTypeRef, options: u32) -> CFTypeRef;
        fn IOHIDManagerSetDeviceMatching(manager: CFTypeRef, matching: CFTypeRef);
        fn IOHIDManagerCopyDevices(manager: CFTypeRef) -> CFTypeRef;
        fn IOHIDDeviceGetProperty(device: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn IOHIDDeviceOpen(device: CFTypeRef, options: u32) -> IOReturn;
        fn IOHIDDeviceClose(device: CFTypeRef, options: u32) -> IOReturn;
        fn IOHIDDeviceScheduleWithRunLoop(device: CFTypeRef, run_loop: CFTypeRef, mode: CFTypeRef);
        fn IOHIDDeviceUnscheduleFromRunLoop(
            device: CFTypeRef,
            run_loop: CFTypeRef,
            mode: CFTypeRef,
        );
        fn IOHIDDeviceRegisterInputReportCallback(
            device: CFTypeRef,
            report: *mut u8,
            length: CFIndex,
            callback: Option<ReportCallback>,
            context: *mut c_void,
        );
        fn IOHIDDeviceSetReport(
            device: CFTypeRef,
            kind: u32,
            id: CFIndex,
            report: *const u8,
            length: CFIndex,
        ) -> IOReturn;
    }

    /// A CFString for `s`, which the caller releases.
    fn cf_string(s: &CStr) -> CFTypeRef {
        // SAFETY: `s` is NUL-terminated, and a null allocator means the
        // default one.
        unsafe {
            CFStringCreateWithCString(std::ptr::null(), s.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        }
    }

    /// A CoreFoundation reference this code owns, released on drop.
    struct Retained(CFTypeRef);

    impl Drop for Retained {
        fn drop(&mut self) {
            // SAFETY: only built from a reference this code retained, and
            // released once, here.
            unsafe { CFRelease(self.0) };
        }
    }

    /// The micro:bits IOKit can see, by serial number.
    fn devices() -> Result<Vec<(String, Retained)>, String> {
        // SAFETY: plain CoreFoundation and IOKit calls on objects this
        // function creates, each checked for null before use. The manager
        // and the set are released here; the devices in it are retained
        // first, so they outlive both.
        unsafe {
            let manager = IOHIDManagerCreate(std::ptr::null(), 0);
            if manager.is_null() {
                return Err("IOKit wouldn't make a HID manager".into());
            }
            let matching = CFDictionaryCreateMutable(
                std::ptr::null(),
                2,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            );
            for (key, value) in [(c"VendorID", VENDOR), (c"ProductID", PRODUCT)] {
                let key = cf_string(key);
                let value = i32::from(value);
                let number = CFNumberCreate(
                    std::ptr::null(),
                    K_CF_NUMBER_SINT32,
                    (&raw const value).cast(),
                );
                CFDictionarySetValue(matching, key, number);
                CFRelease(key);
                CFRelease(number);
            }
            IOHIDManagerSetDeviceMatching(manager, matching);
            CFRelease(matching);

            let set = IOHIDManagerCopyDevices(manager);
            let mut found = Vec::new();
            if !set.is_null() {
                let count = usize::try_from(CFSetGetCount(set)).unwrap_or(0);
                let mut refs = vec![std::ptr::null(); count];
                CFSetGetValues(set, refs.as_mut_ptr());
                let key = cf_string(c"SerialNumber");
                for device in refs {
                    let serial = IOHIDDeviceGetProperty(device, key);
                    let mut buffer = [0 as c_char; 128];
                    let text = if !serial.is_null()
                        && CFGetTypeID(serial) == CFStringGetTypeID()
                        && CFStringGetCString(
                            serial,
                            buffer.as_mut_ptr(),
                            buffer.len() as CFIndex,
                            K_CF_STRING_ENCODING_UTF8,
                        ) != 0
                    {
                        CStr::from_ptr(buffer.as_ptr())
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        String::new()
                    };
                    found.push((text, Retained(CFRetain(device))));
                }
                CFRelease(key);
                CFRelease(set);
            }
            CFRelease(manager);
            found.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(found)
        }
    }

    pub fn list() -> Result<Vec<Probe>, String> {
        Ok(devices()?
            .into_iter()
            .map(|(serial, _)| Probe { serial })
            .collect())
    }

    /// What the input report callback writes into. Boxed, so its address
    /// stays put while IOKit holds it.
    struct Inbox {
        buffer: [u8; REPORT],
        reports: VecDeque<Vec<u8>>,
    }

    extern "C" fn received(
        context: *mut c_void,
        result: IOReturn,
        _sender: *mut c_void,
        _kind: u32,
        _id: u32,
        report: *mut u8,
        length: CFIndex,
    ) {
        if result != K_IORETURN_SUCCESS || context.is_null() || report.is_null() {
            return;
        }
        // SAFETY: `context` is the Inbox that Hid::open registered and Hid
        // owns until it unregisters it, and `report` points at `length`
        // bytes of that Inbox's buffer.
        unsafe {
            let inbox = &mut *context.cast::<Inbox>();
            let length = usize::try_from(length).unwrap_or(0).min(REPORT);
            let bytes = std::slice::from_raw_parts(report, length).to_vec();
            inbox.reports.push_back(bytes);
        }
    }

    pub struct Hid {
        // Dropped after Hid's own Drop has closed it.
        device: Retained,
        run_loop: CFTypeRef,
        inbox: Box<Inbox>,
    }

    impl Hid {
        pub fn open(probe: &Probe) -> Result<Self, String> {
            let (_, device) = devices()?
                .into_iter()
                .find(|(serial, _)| *serial == probe.serial)
                .ok_or_else(|| format!("the micro:bit {} has gone", probe.serial))?;
            let mut inbox = Box::new(Inbox {
                buffer: [0; REPORT],
                reports: VecDeque::new(),
            });
            // SAFETY: `device` is a retained IOHIDDevice. The callback's
            // buffer and context both point into `inbox`, which the Hid
            // keeps alive and unregisters in Drop before freeing.
            unsafe {
                let opened = IOHIDDeviceOpen(device.0, 0);
                if opened != K_IORETURN_SUCCESS {
                    return Err(format!(
                        "macOS wouldn't open the micro:bit's HID interface (IOReturn {opened:#x})"
                    ));
                }
                let run_loop = CFRunLoopGetCurrent();
                IOHIDDeviceScheduleWithRunLoop(device.0, run_loop, kCFRunLoopDefaultMode);
                let context = (&raw mut *inbox).cast::<c_void>();
                IOHIDDeviceRegisterInputReportCallback(
                    device.0,
                    inbox.buffer.as_mut_ptr(),
                    REPORT as CFIndex,
                    Some(received),
                    context,
                );
                let mut hid = Hid {
                    device,
                    run_loop,
                    inbox,
                };
                // Anything left over from a previous run.
                hid.pump(Duration::from_millis(20));
                hid.drain();
                Ok(hid)
            }
        }

        /// Throws away every report that's arrived, including any the run
        /// loop hasn't handed to the callback yet. A reply that turns up
        /// after its command timed out would otherwise answer the next one.
        fn drain(&mut self) {
            for _ in 0..64 {
                // SAFETY: as in pump. A zero timeout handles what's
                // already pending and returns.
                let ran = unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, 1) };
                if ran != K_CF_RUN_LOOP_RUN_HANDLED_SOURCE {
                    break;
                }
            }
            self.inbox.reports.clear();
        }

        fn pump(&mut self, timeout: Duration) {
            let deadline = Instant::now() + timeout;
            while self.inbox.reports.is_empty() && Instant::now() < deadline {
                // SAFETY: runs this thread's run loop, which the device is
                // scheduled on, so the callback can fire.
                unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 1) };
            }
        }
    }

    impl crate::dap::Link for Hid {
        fn exchange(&mut self, command: &[u8]) -> Result<Vec<u8>, String> {
            let report = report(command)?;
            self.drain();
            // SAFETY: an open device and a 64-byte buffer that outlives the
            // call.
            let sent = unsafe {
                IOHIDDeviceSetReport(
                    self.device.0,
                    K_IOHID_REPORT_TYPE_OUTPUT,
                    0,
                    report.as_ptr(),
                    REPORT as CFIndex,
                )
            };
            if sent != K_IORETURN_SUCCESS {
                return Err(format!(
                    "sending to the micro:bit failed (IOReturn {sent:#x})"
                ));
            }
            self.pump(TIMEOUT);
            self.inbox.reports.pop_front().ok_or_else(|| {
                format!("the micro:bit didn't answer within {} s", TIMEOUT.as_secs())
            })
        }
    }

    impl Drop for Hid {
        fn drop(&mut self) {
            let device = self.device.0;
            // SAFETY: undoes what open did, in reverse, on the same device.
            // `self.device` releases it once this returns.
            unsafe {
                IOHIDDeviceRegisterInputReportCallback(
                    device,
                    self.inbox.buffer.as_mut_ptr(),
                    REPORT as CFIndex,
                    None,
                    std::ptr::null_mut(),
                );
                IOHIDDeviceUnscheduleFromRunLoop(device, self.run_loop, kCFRunLoopDefaultMode);
                IOHIDDeviceClose(device, 0);
            }
        }
    }
}
