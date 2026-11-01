use eframe::egui;

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Down,
    Move,
    Up,
    Cancel,
}
#[derive(Clone, Copy, Debug)]
pub struct PenSample {
    pub position: egui::Pos2,
    pub pressure: Option<f32>,
    pub tilt: [Option<f32>; 2],
    pub eraser: bool,
    pub barrel: bool,
    pub contact: bool,
    pub phase: Phase,
}

#[cfg(any(target_os = "linux", test))]
mod pen_mapping {
    pub(super) fn valuator_value(mask: &[u32], values: &[f64], number: u16) -> Option<f64> {
        let axis = usize::from(number);
        let word = axis / 32;
        let bit = axis % 32;
        if !mask.get(word).is_some_and(|value| value & (1 << bit) != 0) {
            return None;
        }
        let value_index = mask
            .iter()
            .take(word)
            .map(|value| value.count_ones() as usize)
            .sum::<usize>()
            + (mask[word] & ((1 << bit) - 1)).count_ones() as usize;
        values.get(value_index).copied()
    }

    pub(super) fn normalize_axis(value: f64, min: f64, max: f64) -> f32 {
        if max <= min {
            return 0.0;
        }
        ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
    }

    pub(super) fn normalize_tilt(value: f64, min: f64, max: f64) -> f32 {
        if max <= min {
            return 0.0;
        }
        (value.clamp(min, max) as f32).clamp(-90.0, 90.0)
    }

    pub(super) fn fixed_16_16(value: i32) -> f32 {
        value as f32 / 65_536.0
    }
}

#[cfg(test)]
mod pen_mapping_tests {
    use super::pen_mapping::*;

    #[test]
    fn maps_sparse_valuator_masks_to_pressure_and_tilt_values() {
        let mask = [0b1010];
        let values = [20.0, -30.0];

        assert_eq!(
            normalize_axis(valuator_value(&mask, &values, 1).unwrap(), 0.0, 100.0),
            0.2
        );
        assert_eq!(
            normalize_tilt(valuator_value(&mask, &values, 3).unwrap(), -60.0, 60.0),
            -30.0
        );
        assert_eq!(valuator_value(&mask, &values, 0), None);
    }

    #[test]
    fn clamps_pressure_and_tilt_and_handles_invalid_axis_ranges() {
        assert_eq!(normalize_axis(125.0, 0.0, 100.0), 1.0);
        assert_eq!(normalize_axis(5.0, 7.0, 7.0), 0.0);
        assert_eq!(normalize_tilt(120.0, -120.0, 120.0), 90.0);
    }

    #[test]
    fn converts_xinput_window_coordinates_from_fixed_point() {
        assert_eq!(fixed_16_16(1 << 16), 1.0);
        assert_eq!(fixed_16_16(-(2 << 16)), -2.0);
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::cell::RefCell;
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::ScreenToClient,
        UI::{Input::Pointer::*, WindowsAndMessaging::*},
    };

    struct State {
        previous: WNDPROC,
        samples: Vec<PenSample>,
        context: egui::Context,
    }
    thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }
    pub struct PenInput {
        hwnd: HWND,
        previous: isize,
    }

    impl PenInput {
        pub fn install(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
            let handle = cc.window_handle().map_err(|e| e.to_string())?;
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return Err("Not a Win32 window".into());
            };
            let hwnd = handle.hwnd.get() as HWND;
            // SAFETY: the HWND belongs to this UI thread. The old procedure is
            // retained and every message is forwarded. Drop restores the hook.
            unsafe {
                let previous =
                    SetWindowLongPtrW(hwnd, GWLP_WNDPROC, procedure as *const () as isize);
                if previous == 0 {
                    return Err("Could not attach Windows Ink listener".into());
                }
                let proc: WNDPROC = Some(std::mem::transmute::<
                    isize,
                    unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
                >(previous));
                STATE.with(|s| {
                    *s.borrow_mut() = Some(State {
                        previous: proc,
                        samples: Vec::new(),
                        context: cc.egui_ctx.clone(),
                    })
                });
                Ok(Self { hwnd, previous })
            }
        }
        pub fn drain(&self) -> Vec<PenSample> {
            STATE.with(|s| {
                s.borrow_mut()
                    .as_mut()
                    .map(|s| std::mem::take(&mut s.samples))
                    .unwrap_or_default()
            })
        }
    }
    impl Drop for PenInput {
        fn drop(&mut self) {
            // SAFETY: same UI thread; never overwrite a hook installed after us.
            unsafe {
                if IsWindow(self.hwnd) != 0
                    && GetWindowLongPtrW(self.hwnd, GWLP_WNDPROC) == procedure as *const () as isize
                {
                    SetWindowLongPtrW(self.hwnd, GWLP_WNDPROC, self.previous);
                }
            }
            STATE.with(|s| *s.borrow_mut() = None);
        }
    }

    unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        let previous = STATE.with(|s| s.borrow().as_ref().and_then(|s| s.previous));
        if matches!(msg, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
            let id = (w & 0xffff) as u32;
            let mut latest: POINTER_PEN_INFO = std::mem::zeroed();
            if GetPointerPenInfo(id, &mut latest) != 0 {
                let mut count = latest.pointerInfo.historyCount.clamp(1, 4096);
                let mut history = vec![latest; count as usize];
                if GetPointerPenInfoHistory(id, &mut count, history.as_mut_ptr()) == 0 {
                    history.clear();
                    history.push(latest);
                } else {
                    history.truncate(count as usize);
                }
                // Windows gives newest first. Preserve chronological input order.
                STATE.with(|state| {
                    if let Some(state) = state.borrow_mut().as_mut() {
                        for info in history.into_iter().rev() {
                            let flags = info.pointerInfo.pointerFlags;
                            let mut position = info.pointerInfo.ptPixelLocation;
                            if ScreenToClient(hwnd, &mut position) == 0 {
                                continue;
                            }
                            let phase = if flags & POINTER_FLAG_CANCELED != 0 {
                                Phase::Cancel
                            } else if flags & POINTER_FLAG_DOWN != 0 {
                                Phase::Down
                            } else if flags & POINTER_FLAG_UP != 0 {
                                Phase::Up
                            } else {
                                Phase::Move
                            };
                            state.samples.push(PenSample {
                                position: egui::pos2(position.x as f32, position.y as f32),
                                pressure: (info.penMask & PEN_MASK_PRESSURE != 0)
                                    .then_some((info.pressure as f32 / 1024.0).clamp(0.0, 1.0)),
                                tilt: [
                                    (info.penMask & PEN_MASK_TILT_X != 0)
                                        .then_some(info.tiltX as f32),
                                    (info.penMask & PEN_MASK_TILT_Y != 0)
                                        .then_some(info.tiltY as f32),
                                ],
                                eraser: info.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0,
                                barrel: info.penFlags & PEN_FLAG_BARREL != 0,
                                contact: flags & POINTER_FLAG_INCONTACT != 0,
                                phase,
                            });
                        }
                        if state.samples.len() > 32768 {
                            state.samples.clear();
                            state.samples.push(PenSample {
                                position: egui::Pos2::ZERO,
                                pressure: None,
                                tilt: [None; 2],
                                eraser: false,
                                barrel: false,
                                contact: false,
                                phase: Phase::Cancel,
                            });
                        }
                        state.context.request_repaint();
                    }
                });
            }
        }
        if matches!(msg, WM_KILLFOCUS | WM_POINTERCAPTURECHANGED) {
            STATE.with(|s| {
                if let Some(s) = s.borrow_mut().as_mut() {
                    s.samples.push(PenSample {
                        position: egui::Pos2::ZERO,
                        pressure: None,
                        tilt: [None; 2],
                        eraser: false,
                        barrel: false,
                        contact: false,
                        phase: Phase::Cancel,
                    });
                }
            });
        }
        // Do not consume: egui still needs pen-promoted mouse events for widgets.
        CallWindowProcW(previous, hwnd, msg, w, l)
    }
}
#[cfg(windows)]
pub use native::PenInput;

#[cfg(target_os = "linux")]
mod linux_x11 {
    use super::pen_mapping::{
        fixed_16_16, normalize_axis, normalize_tilt, valuator_value as map_valuator_value,
    };
    use super::*;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{
        collections::{HashMap, HashSet},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::Duration,
    };
    use x11rb::{
        connection::Connection,
        protocol::{
            xinput::ConnectionExt as XInputConnectionExt,
            xinput::{self, DeviceClassData, EventMask, XIEventMask},
            xproto::ConnectionExt as XprotoConnectionExt,
            Event,
        },
        rust_connection::RustConnection,
    };

    #[derive(Clone, Copy, Debug, Default)]
    struct Axes {
        pressure: Option<(u16, f64, f64)>,
        tilt_x: Option<(u16, f64, f64)>,
        tilt_y: Option<(u16, f64, f64)>,
        eraser: bool,
    }

    #[derive(Clone, Copy)]
    struct SampleState {
        eraser: bool,
        barrel: bool,
        contact: bool,
        phase: Phase,
    }

    pub struct PenInput {
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        samples: Arc<Mutex<Vec<PenSample>>>,
    }

    impl PenInput {
        pub fn install(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
            let handle = cc.window_handle().map_err(|e| e.to_string())?;
            let window = match handle.as_raw() {
                RawWindowHandle::Xlib(handle) => handle.window as u32,
                RawWindowHandle::Xcb(handle) => handle.window.get(),
                RawWindowHandle::Wayland(_) => {
                    return Err(
                        "Native pen input currently supports X11 only; Wayland is not yet supported."
                            .into(),
                    );
                }
                _ => return Err("Native pen input requires an X11 window.".into()),
            };

            let (connection, _) = x11rb::connect(None)
                .map_err(|error| format!("Could not connect to the X server: {error}"))?;
            connection
                .xinput_xi_query_version(2, 0)
                .map_err(|error| format!("XInput2 is unavailable: {error}"))?
                .reply()
                .map_err(|error| format!("XInput2 is unavailable: {error}"))?;

            let masks = [EventMask {
                deviceid: 0, // XIAllDevices
                mask: vec![
                    XIEventMask::DEVICE_CHANGED
                        | XIEventMask::BUTTON_PRESS
                        | XIEventMask::BUTTON_RELEASE
                        | XIEventMask::MOTION
                        | XIEventMask::FOCUS_OUT
                        | XIEventMask::HIERARCHY,
                ],
            }];
            connection
                .xinput_xi_select_events(window, &masks)
                .map_err(|error| format!("Could not select XInput2 pen events: {error}"))?
                .check()
                .map_err(|error| format!("Could not select XInput2 pen events: {error}"))?;
            connection
                .flush()
                .map_err(|error| format!("Could not start XInput2 event stream: {error}"))?;

            let stop = Arc::new(AtomicBool::new(false));
            let samples = Arc::new(Mutex::new(Vec::new()));
            let worker_stop = stop.clone();
            let worker_samples = samples.clone();
            let context = cc.egui_ctx.clone();
            let worker = thread::Builder::new()
                .name("sketch3d-xinput2-pen".into())
                .spawn(move || run_listener(connection, worker_stop, worker_samples, context))
                .map_err(|error| format!("Could not start XInput2 listener: {error}"))?;

            Ok(Self {
                stop,
                worker: Some(worker),
                samples,
            })
        }

        pub fn drain(&self) -> Vec<PenSample> {
            self.samples
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .drain(..)
                .collect()
        }
    }

    impl Drop for PenInput {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn run_listener(
        connection: RustConnection,
        stop: Arc<AtomicBool>,
        samples: Arc<Mutex<Vec<PenSample>>>,
        context: egui::Context,
    ) {
        let mut axes = match query_tablet_axes(&connection) {
            Ok(axes) => axes,
            Err(_) => return,
        };
        let mut contact = HashSet::new();

        while !stop.load(Ordering::Acquire) {
            let event = match connection.poll_for_event() {
                Ok(Some(event)) => event,
                Ok(None) => {
                    thread::sleep(Duration::from_millis(8));
                    continue;
                }
                Err(_) => break,
            };

            match event {
                Event::XinputDeviceChanged(_) | Event::XinputHierarchy(_) => {
                    contact.clear();
                    push_sample(&samples, canceled_sample());
                    axes = query_tablet_axes(&connection).unwrap_or_default();
                    context.request_repaint();
                }
                Event::XinputFocusOut(_) => {
                    if !contact.is_empty() {
                        contact.clear();
                        push_sample(&samples, canceled_sample());
                        context.request_repaint();
                    }
                }
                Event::XinputButtonPress(event) => {
                    let source = event.sourceid;
                    let Some(device_axes) = axes.get(&source).copied() else {
                        continue;
                    };
                    let position = event_position(event.event_x, event.event_y);
                    let barrel = event.detail == 2 || event.detail == 3;
                    if event.detail == 1 {
                        contact.insert(source);
                    }
                    push_sample(
                        &samples,
                        sample_from_event(
                            position,
                            &event.valuator_mask,
                            &event.axisvalues,
                            device_axes,
                            SampleState {
                                eraser: device_axes.eraser,
                                barrel,
                                contact: contact.contains(&source),
                                phase: if event.detail == 1 {
                                    Phase::Down
                                } else {
                                    Phase::Move
                                },
                            },
                        ),
                    );
                    context.request_repaint();
                }
                Event::XinputButtonRelease(event) => {
                    let source = event.sourceid;
                    let Some(device_axes) = axes.get(&source).copied() else {
                        continue;
                    };
                    if event.detail == 1 {
                        contact.remove(&source);
                    }
                    push_sample(
                        &samples,
                        sample_from_event(
                            event_position(event.event_x, event.event_y),
                            &event.valuator_mask,
                            &event.axisvalues,
                            device_axes,
                            SampleState {
                                eraser: device_axes.eraser,
                                barrel: false,
                                contact: contact.contains(&source),
                                phase: if event.detail == 1 {
                                    Phase::Up
                                } else {
                                    Phase::Move
                                },
                            },
                        ),
                    );
                    context.request_repaint();
                }
                Event::XinputMotion(event) => {
                    let source = event.sourceid;
                    let Some(device_axes) = axes.get(&source).copied() else {
                        continue;
                    };
                    push_sample(
                        &samples,
                        sample_from_event(
                            event_position(event.event_x, event.event_y),
                            &event.valuator_mask,
                            &event.axisvalues,
                            device_axes,
                            SampleState {
                                eraser: device_axes.eraser,
                                barrel: button_is_down(&event.button_mask, 2)
                                    || button_is_down(&event.button_mask, 3),
                                contact: contact.contains(&source),
                                phase: Phase::Move,
                            },
                        ),
                    );
                    context.request_repaint();
                }
                _ => {}
            }
        }
    }

    fn query_tablet_axes(
        connection: &RustConnection,
    ) -> Result<HashMap<xinput::DeviceId, Axes>, String> {
        let devices = connection
            .xinput_xi_query_device(0u16) // XIAllDevices
            .map_err(|error| error.to_string())?
            .reply()
            .map_err(|error| error.to_string())?
            .infos;
        let mut axes = HashMap::new();

        for device in devices.into_iter().filter(|device| device.enabled) {
            let mut device_axes = Axes {
                eraser: String::from_utf8_lossy(&device.name)
                    .to_ascii_lowercase()
                    .contains("eraser"),
                ..Axes::default()
            };
            for class in device.classes {
                let DeviceClassData::Valuator(valuator) = class.data else {
                    continue;
                };
                let label = connection
                    .get_atom_name(valuator.label)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .map(|reply| String::from_utf8_lossy(&reply.name).to_ascii_lowercase())
                    .unwrap_or_default();
                let range = (valuator.number, fp3232(valuator.min), fp3232(valuator.max));
                if label.contains("pressure") {
                    device_axes.pressure = Some(range);
                } else if label.contains("tilt") && label.ends_with('x') {
                    device_axes.tilt_x = Some(range);
                } else if label.contains("tilt") && label.ends_with('y') {
                    device_axes.tilt_y = Some(range);
                }
            }
            if device_axes.pressure.is_some()
                || device_axes.tilt_x.is_some()
                || device_axes.tilt_y.is_some()
            {
                axes.insert(device.deviceid, device_axes);
            }
        }
        Ok(axes)
    }

    fn sample_from_event(
        position: egui::Pos2,
        valuator_mask: &[u32],
        axis_values: &[xinput::Fp3232],
        axes: Axes,
        state: SampleState,
    ) -> PenSample {
        let values = axis_values.iter().copied().map(fp3232).collect::<Vec<_>>();
        PenSample {
            position,
            pressure: axes.pressure.and_then(|axis| {
                map_valuator_value(valuator_mask, &values, axis.0)
                    .map(|value| normalize_axis(value, axis.1, axis.2))
            }),
            tilt: [
                axes.tilt_x.and_then(|axis| {
                    map_valuator_value(valuator_mask, &values, axis.0)
                        .map(|value| normalize_tilt(value, axis.1, axis.2))
                }),
                axes.tilt_y.and_then(|axis| {
                    map_valuator_value(valuator_mask, &values, axis.0)
                        .map(|value| normalize_tilt(value, axis.1, axis.2))
                }),
            ],
            eraser: state.eraser,
            barrel: state.barrel,
            contact: state.contact,
            phase: state.phase,
        }
    }

    fn fp3232(value: xinput::Fp3232) -> f64 {
        f64::from(value.integral) + f64::from(value.frac) / 4_294_967_296.0
    }

    fn event_position(x: xinput::Fp1616, y: xinput::Fp1616) -> egui::Pos2 {
        egui::pos2(fixed_16_16(x), fixed_16_16(y))
    }

    fn button_is_down(mask: &[u32], button: usize) -> bool {
        let bit = button.saturating_sub(1);
        mask.get(bit / 32)
            .is_some_and(|word| word & (1 << (bit % 32)) != 0)
    }

    fn push_sample(samples: &Mutex<Vec<PenSample>>, sample: PenSample) {
        let mut samples = samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if samples.len() >= 32_768 {
            samples.clear();
            samples.push(canceled_sample());
        } else {
            samples.push(sample);
        }
    }

    fn canceled_sample() -> PenSample {
        PenSample {
            position: egui::Pos2::ZERO,
            pressure: None,
            tilt: [None, None],
            eraser: false,
            barrel: false,
            contact: false,
            phase: Phase::Cancel,
        }
    }
}
#[cfg(target_os = "linux")]
pub use linux_x11::PenInput;

#[cfg(target_os = "macos")]
mod macos_appkit {
    use super::*;
    use block2::RcBlock;
    use objc2::{rc::Retained, runtime::AnyObject, MainThreadMarker};
    use objc2_app_kit::{
        NSEvent, NSEventMask, NSEventSubtype, NSEventType, NSPointingDeviceType, NSView,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{
        marker::PhantomData,
        ptr::NonNull,
        rc::Rc,
        sync::{Arc, Mutex},
    };

    pub struct PenInput {
        monitor: Option<Retained<AnyObject>>,
        samples: Arc<Mutex<Vec<PenSample>>>,
        _main_thread_only: PhantomData<Rc<()>>,
    }

    impl PenInput {
        pub fn install(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
            let handle = cc.window_handle().map_err(|e| e.to_string())?;
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return Err("Native pen input requires a macOS AppKit window.".into());
            };
            let _main_thread = MainThreadMarker::new().ok_or_else(|| {
                "AppKit pen input must be installed on the main thread.".to_owned()
            })?;
            let view_ptr = handle.ns_view.cast::<NSView>();
            // SAFETY: eframe's AppKit window handle guarantees this is a live NSView
            // on the main thread; the app owns the view for at least the monitor's life.
            let view = unsafe { view_ptr.as_ref() };
            let window = view
                .window()
                .ok_or_else(|| "The AppKit view is not attached to a window.".to_owned())?;
            let window_number = window.windowNumber();
            let view_address = view_ptr.as_ptr() as usize;
            let samples = Arc::new(Mutex::new(Vec::new()));
            let contact = Arc::new(Mutex::new(false));
            let output = samples.clone();
            let contact_state = contact.clone();
            let context = cc.egui_ctx.clone();
            let mask = NSEventMask::LeftMouseDown
                | NSEventMask::LeftMouseUp
                | NSEventMask::LeftMouseDragged
                | NSEventMask::TabletPoint
                | NSEventMask::TabletProximity
                | NSEventMask::MouseCancelled;
            let handler = RcBlock::new(move |event_ptr: NonNull<NSEvent>| -> *mut NSEvent {
                // AppKit invokes local monitors on the app's main event thread.
                let event = unsafe { event_ptr.as_ref() };
                if event.windowNumber() != window_number {
                    return event_ptr.as_ptr();
                }
                let event_type = event.r#type();
                if event_type == NSEventType::MouseCancelled
                    || (event_type == NSEventType::TabletProximity && !event.isEnteringProximity())
                {
                    *contact_state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = false;
                    push_sample(&output, canceled_sample());
                    context.request_repaint();
                    return event_ptr.as_ptr();
                }

                let tablet_event = event_type == NSEventType::TabletPoint
                    || (event.subtype() == NSEventSubtype::TabletPoint
                        && matches!(
                            event.pointingDeviceType(),
                            NSPointingDeviceType::Pen | NSPointingDeviceType::Eraser
                        ));
                if !tablet_event {
                    return event_ptr.as_ptr();
                }

                let view = unsafe { &*(view_address as *const NSView) };
                let bounds = view.bounds();
                let point = view.convertPoint_fromView(event.locationInWindow(), None);
                let position = egui::pos2(
                    point.x as f32,
                    (bounds.origin.y + bounds.size.height - point.y) as f32,
                );
                let buttons = event.buttonMask();
                let tip_down = buttons.contains(objc2_app_kit::NSEventButtonMask::PenTip);
                let mut in_contact = contact_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let phase = if event_type == NSEventType::LeftMouseDown {
                    *in_contact = true;
                    Phase::Down
                } else if event_type == NSEventType::LeftMouseUp {
                    *in_contact = false;
                    Phase::Up
                } else if tip_down && !*in_contact {
                    *in_contact = true;
                    Phase::Down
                } else if !tip_down && *in_contact && event_type == NSEventType::TabletPoint {
                    *in_contact = false;
                    Phase::Up
                } else {
                    Phase::Move
                };
                let tilt = event.tilt();
                push_sample(
                    &output,
                    PenSample {
                        position,
                        pressure: Some(event.pressure().clamp(0.0, 1.0)),
                        tilt: [
                            Some((tilt.x as f32 * 90.0).clamp(-90.0, 90.0)),
                            Some((tilt.y as f32 * 90.0).clamp(-90.0, 90.0)),
                        ],
                        eraser: event.pointingDeviceType() == NSPointingDeviceType::Eraser,
                        barrel: buttons.contains(objc2_app_kit::NSEventButtonMask::PenLowerSide)
                            || buttons.contains(objc2_app_kit::NSEventButtonMask::PenUpperSide),
                        contact: *in_contact,
                        phase,
                    },
                );
                context.request_repaint();
                event_ptr.as_ptr()
            });
            // SAFETY: the local monitor is installed on AppKit's main thread. The
            // handler returns the original event so winit and widgets still receive it.
            let monitor =
                unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }
                    .ok_or_else(|| {
                        "AppKit could not install the tablet event monitor.".to_owned()
                    })?;

            Ok(Self {
                monitor: Some(monitor),
                samples,
                _main_thread_only: PhantomData,
            })
        }

        pub fn drain(&self) -> Vec<PenSample> {
            self.samples
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .drain(..)
                .collect()
        }
    }

    impl Drop for PenInput {
        fn drop(&mut self) {
            if MainThreadMarker::new().is_some() {
                if let Some(monitor) = self.monitor.take() {
                    // SAFETY: the token came from this AppKit event monitor and Drop
                    // normally runs on the same main thread as installation.
                    unsafe { NSEvent::removeMonitor(&monitor) };
                }
            }
        }
    }

    fn push_sample(samples: &Mutex<Vec<PenSample>>, sample: PenSample) {
        let mut samples = samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if samples.len() >= 32_768 {
            samples.clear();
            samples.push(canceled_sample());
        } else {
            samples.push(sample);
        }
    }

    fn canceled_sample() -> PenSample {
        PenSample {
            position: egui::Pos2::ZERO,
            pressure: None,
            tilt: [None, None],
            eraser: false,
            barrel: false,
            contact: false,
            phase: Phase::Cancel,
        }
    }
}
#[cfg(target_os = "macos")]
pub use macos_appkit::PenInput;

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub struct PenInput;
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
impl PenInput {
    pub fn install(_: &eframe::CreationContext<'_>) -> Result<Self, String> {
        Err(
            "Native pen backend currently implemented on Windows only; mouse drawing is available."
                .into(),
        )
    }
    pub fn drain(&self) -> Vec<PenSample> {
        Vec::new()
    }
}
