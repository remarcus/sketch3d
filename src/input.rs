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

    pub(super) fn update_axis_cache(
        cached: &mut Option<f32>,
        mask: &[u32],
        values: &[f64],
        axis: Option<(u16, f64, f64)>,
        normalize: fn(f64, f64, f64) -> f32,
    ) {
        if let Some((number, min, max)) = axis {
            if let Some(value) = valuator_value(mask, values, number) {
                *cached = Some(normalize(value, min, max));
            }
        }
    }

    pub(super) fn fixed_16_16(value: i32) -> f32 {
        value as f32 / 65_536.0
    }

    pub(super) fn wayland_pressure(value: u32) -> f32 {
        (value as f32 / 65_535.0).clamp(0.0, 1.0)
    }

    pub(super) fn wayland_tilt(value: f64) -> f32 {
        if value.is_finite() {
            (value as f32).clamp(-90.0, 90.0)
        } else {
            0.0
        }
    }

    pub(super) fn wayland_position(x: f64, y: f64, scale: f32) -> eframe::egui::Pos2 {
        eframe::egui::pos2((x * f64::from(scale)) as f32, (y * f64::from(scale)) as f32)
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
    fn keeps_last_axis_values_when_xinput_events_omit_unchanged_valuators() {
        let mut pressure = None;
        update_axis_cache(
            &mut pressure,
            &[1 << 3],
            &[42.0],
            Some((3, 0.0, 100.0)),
            normalize_axis,
        );
        assert_eq!(pressure, Some(0.42));

        update_axis_cache(
            &mut pressure,
            &[1],
            &[7.0],
            Some((3, 0.0, 100.0)),
            normalize_axis,
        );
        assert_eq!(pressure, Some(0.42));
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

    #[test]
    fn normalizes_wayland_pressure_and_clamps_tilt() {
        assert_eq!(wayland_pressure(0), 0.0);
        assert_eq!(wayland_pressure(65_535), 1.0);
        assert_eq!(wayland_tilt(120.0), 90.0);
        assert_eq!(wayland_tilt(-120.0), -90.0);
        assert_eq!(wayland_tilt(f64::NAN), 0.0);
    }

    #[test]
    fn converts_wayland_surface_coordinates_to_pixel_coordinates() {
        assert_eq!(
            wayland_position(12.5, 4.0, 2.0),
            eframe::egui::pos2(25.0, 8.0)
        );
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
    use super::pen_mapping::{fixed_16_16, normalize_axis, normalize_tilt, update_axis_cache};
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

    #[derive(Clone, Copy, Default)]
    struct DynamicAxes {
        pressure: Option<f32>,
        tilt_x: Option<f32>,
        tilt_y: Option<f32>,
    }

    pub struct X11PenInput {
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        samples: Arc<Mutex<Vec<PenSample>>>,
    }

    impl X11PenInput {
        pub(super) fn install(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
            let handle = cc.window_handle().map_err(|e| e.to_string())?;
            let window = match handle.as_raw() {
                RawWindowHandle::Xlib(handle) => handle.window as u32,
                RawWindowHandle::Xcb(handle) => handle.window.get(),
                RawWindowHandle::Wayland(_) => unreachable!("Wayland uses the tablet-v2 backend"),
                _ => return Err("Native pen input requires an X11 window.".into()),
            };

            let (connection, _) = x11rb::connect(None)
                .map_err(|error| format!("Could not connect to the X server: {error}"))?;
            connection
                .xinput_xi_query_version(2, 0)
                .map_err(|error| format!("XInput2 is unavailable: {error}"))?
                .reply()
                .map_err(|error| format!("XInput2 is unavailable: {error}"))?;
            let axes = query_tablet_axes(&connection)
                .map_err(|error| format!("Could not inspect XInput2 tablet axes: {error}"))?;

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
                .spawn(move || run_listener(connection, axes, worker_stop, worker_samples, context))
                .map_err(|error| format!("Could not start XInput2 listener: {error}"))?;

            Ok(Self {
                stop,
                worker: Some(worker),
                samples,
            })
        }

        pub(super) fn drain(&self) -> Vec<PenSample> {
            self.samples
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .drain(..)
                .collect()
        }
    }

    impl Drop for X11PenInput {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn run_listener(
        connection: RustConnection,
        mut axes: HashMap<xinput::DeviceId, Axes>,
        stop: Arc<AtomicBool>,
        samples: Arc<Mutex<Vec<PenSample>>>,
        context: egui::Context,
    ) {
        let mut contact = HashSet::new();
        let mut dynamic_axes = HashMap::<xinput::DeviceId, DynamicAxes>::new();

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
                    dynamic_axes.clear();
                    push_sample(&samples, canceled_sample());
                    if let Ok(updated_axes) = query_tablet_axes(&connection) {
                        axes = updated_axes;
                    }
                    context.request_repaint();
                }
                Event::XinputFocusOut(_) => {
                    dynamic_axes.clear();
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
                            source,
                            position,
                            &event.valuator_mask,
                            &event.axisvalues,
                            device_axes,
                            &mut dynamic_axes,
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
                            source,
                            event_position(event.event_x, event.event_y),
                            &event.valuator_mask,
                            &event.axisvalues,
                            device_axes,
                            &mut dynamic_axes,
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
                    if event.detail == 1 {
                        dynamic_axes.remove(&source);
                    }
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
                            source,
                            event_position(event.event_x, event.event_y),
                            &event.valuator_mask,
                            &event.axisvalues,
                            device_axes,
                            &mut dynamic_axes,
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
        source: xinput::DeviceId,
        position: egui::Pos2,
        valuator_mask: &[u32],
        axis_values: &[xinput::Fp3232],
        axes: Axes,
        dynamic_axes: &mut HashMap<xinput::DeviceId, DynamicAxes>,
        state: SampleState,
    ) -> PenSample {
        let values = axis_values.iter().copied().map(fp3232).collect::<Vec<_>>();
        let dynamic = dynamic_axes.entry(source).or_default();
        update_axis_cache(
            &mut dynamic.pressure,
            valuator_mask,
            &values,
            axes.pressure,
            normalize_axis,
        );
        update_axis_cache(
            &mut dynamic.tilt_x,
            valuator_mask,
            &values,
            axes.tilt_x,
            normalize_tilt,
        );
        update_axis_cache(
            &mut dynamic.tilt_y,
            valuator_mask,
            &values,
            axes.tilt_y,
            normalize_tilt,
        );
        PenSample {
            position,
            pressure: dynamic.pressure,
            tilt: [dynamic.tilt_x, dynamic.tilt_y],
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
mod linux_wayland {
    use super::pen_mapping::{wayland_position, wayland_pressure, wayland_tilt};
    use super::*;
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };
    use wayland_client::{
        backend::{Backend, ObjectId},
        protocol::{wl_registry, wl_seat, wl_surface},
        Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    };
    use wayland_protocols::wp::tablet::zv2::client::{
        zwp_tablet_manager_v2::{self, ZwpTabletManagerV2},
        zwp_tablet_pad_dial_v2::ZwpTabletPadDialV2,
        zwp_tablet_pad_group_v2::{self, ZwpTabletPadGroupV2},
        zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2,
        zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2,
        zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
        zwp_tablet_seat_v2::{self, ZwpTabletSeatV2},
        zwp_tablet_tool_v2::{self, ZwpTabletToolV2},
        zwp_tablet_v2::{self, ZwpTabletV2},
    };

    pub struct PenInput {
        queue: EventQueue<State>,
        state: State,
        connection: Connection,
    }

    struct State {
        surface: wl_surface::WlSurface,
        scale: f32,
        samples: Arc<Mutex<Vec<PenSample>>>,
        manager: Option<ZwpTabletManagerV2>,
        seat: Option<wl_seat::WlSeat>,
        tablet_seat: Option<ZwpTabletSeatV2>,
        tablets: Vec<ZwpTabletV2>,
        tools: Vec<ZwpTabletToolV2>,
        pads: Vec<ZwpTabletPadV2>,
        groups: Vec<ZwpTabletPadGroupV2>,
        rings: Vec<ZwpTabletPadRingV2>,
        strips: Vec<ZwpTabletPadStripV2>,
        dials: Vec<ZwpTabletPadDialV2>,
        context: egui::Context,
    }

    struct Tool {
        inner: Mutex<ToolState>,
    }

    struct ToolState {
        eraser: bool,
        barrel_buttons: HashSet<u32>,
        contact: bool,
        position: Option<(f64, f64)>,
        pressure: Option<f32>,
        tilt: [Option<f32>; 2],
        phase: Phase,
        focused: bool,
    }

    impl Default for Tool {
        fn default() -> Self {
            Self {
                inner: Mutex::new(ToolState {
                    eraser: false,
                    barrel_buttons: HashSet::new(),
                    contact: false,
                    position: None,
                    pressure: None,
                    tilt: [None; 2],
                    phase: Phase::Move,
                    focused: false,
                }),
            }
        }
    }

    impl PenInput {
        pub fn install(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
            let display_handle = cc.display_handle().map_err(|e| e.to_string())?;
            let RawDisplayHandle::Wayland(display) = display_handle.as_raw() else {
                return Err("The Wayland pen backend requires a Wayland display.".into());
            };
            let window_handle = cc.window_handle().map_err(|e| e.to_string())?;
            let RawWindowHandle::Wayland(window) = window_handle.as_raw() else {
                return Err("The Wayland pen backend requires a Wayland surface.".into());
            };
            // SAFETY: winit owns this display for the lifetime of the app and this
            // guest backend is dropped with its surface proxy before winit closes it.
            let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
            let connection = Connection::from_backend(backend);
            // SAFETY: the pointer is the live wl_surface from this same display;
            // ObjectId is only used with the guest connection wrapping that display.
            let surface_id = unsafe {
                ObjectId::from_ptr(
                    wl_surface::WlSurface::interface(),
                    window.surface.as_ptr().cast(),
                )
            }
            .map_err(|error| format!("Could not import the Wayland window surface: {error}"))?;
            let surface = wl_surface::WlSurface::from_id(&connection, surface_id)
                .map_err(|error| format!("Could not manage the Wayland window surface: {error}"))?;
            let queue = connection.new_event_queue();
            let qh = queue.handle();
            connection.display().get_registry(&qh, ());
            connection
                .flush()
                .map_err(|e| format!("Could not initialize tablet-v2: {e}"))?;
            let state = State {
                surface,
                scale: cc.egui_ctx.pixels_per_point(),
                samples: Arc::new(Mutex::new(Vec::new())),
                manager: None,
                seat: None,
                tablet_seat: None,
                tablets: Vec::new(),
                tools: Vec::new(),
                pads: Vec::new(),
                groups: Vec::new(),
                rings: Vec::new(),
                strips: Vec::new(),
                dials: Vec::new(),
                context: cc.egui_ctx.clone(),
            };
            Ok(Self {
                queue,
                state,
                connection,
            })
        }

        pub fn drain(&mut self) -> Vec<PenSample> {
            self.state.scale = self.state.context.pixels_per_point();
            if let Err(error) = self.queue.dispatch_pending(&mut self.state) {
                eprintln!("Wayland tablet event dispatch stopped: {error}");
                push_sample(&self.state.samples, canceled_sample());
            }
            let _ = self.connection.flush();
            self.state
                .samples
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .drain(..)
                .collect()
        }
    }

    impl Drop for PenInput {
        fn drop(&mut self) {
            for proxy in self.state.tools.drain(..) {
                proxy.destroy();
            }
            for proxy in self.state.dials.drain(..) {
                proxy.destroy();
            }
            for proxy in self.state.rings.drain(..) {
                proxy.destroy();
            }
            for proxy in self.state.strips.drain(..) {
                proxy.destroy();
            }
            for proxy in self.state.groups.drain(..) {
                proxy.destroy();
            }
            for proxy in self.state.pads.drain(..) {
                proxy.destroy();
            }
            for proxy in self.state.tablets.drain(..) {
                proxy.destroy();
            }
            if let Some(seat) = self.state.tablet_seat.take() {
                seat.destroy();
            }
            if let Some(manager) = self.state.manager.take() {
                manager.destroy();
            }
            if let Some(seat) = self.state.seat.take() {
                seat.release();
            }
            let _ = self.connection.flush();
        }
    }

    impl State {
        fn maybe_bind_tablet_seat(&mut self, qh: &QueueHandle<Self>) {
            if self.tablet_seat.is_none() {
                if let (Some(manager), Some(seat)) = (&self.manager, &self.seat) {
                    self.tablet_seat = Some(manager.get_tablet_seat(seat, qh, ()));
                }
            }
        }
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for State {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                if interface == "zwp_tablet_manager_v2" && state.manager.is_none() {
                    state.manager = Some(registry.bind::<ZwpTabletManagerV2, _, _>(
                        name,
                        version.min(2),
                        qh,
                        (),
                    ));
                } else if interface == "wl_seat" && state.seat.is_none() {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(8), qh, ()));
                }
                state.maybe_bind_tablet_seat(qh);
            }
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for State {
        fn event(
            _: &mut Self,
            _: &wl_seat::WlSeat,
            _: wl_seat::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ZwpTabletManagerV2, ()> for State {
        fn event(
            _: &mut Self,
            _: &ZwpTabletManagerV2,
            _: zwp_tablet_manager_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ZwpTabletSeatV2, ()> for State {
        fn event(
            state: &mut Self,
            _: &ZwpTabletSeatV2,
            event: zwp_tablet_seat_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zwp_tablet_seat_v2::Event::ToolAdded { id } => state.tools.push(id),
                zwp_tablet_seat_v2::Event::TabletAdded { id } => state.tablets.push(id),
                zwp_tablet_seat_v2::Event::PadAdded { id } => state.pads.push(id),
                _ => {}
            }
        }

        wayland_client::event_created_child!(State, ZwpTabletSeatV2, [
            zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (ZwpTabletToolV2, Tool::default()),
            zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (ZwpTabletV2, ()),
            zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (ZwpTabletPadV2, ()),
        ]);
    }

    impl Dispatch<ZwpTabletV2, ()> for State {
        fn event(
            state: &mut Self,
            proxy: &ZwpTabletV2,
            event: zwp_tablet_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if matches!(event, zwp_tablet_v2::Event::Removed) {
                proxy.destroy();
                state.tablets.retain(|item| item.id() != proxy.id());
            }
        }
    }

    impl Dispatch<ZwpTabletPadV2, ()> for State {
        fn event(
            state: &mut Self,
            proxy: &ZwpTabletPadV2,
            event: zwp_tablet_pad_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if matches!(event, zwp_tablet_pad_v2::Event::Removed) {
                proxy.destroy();
                state.pads.retain(|item| item.id() != proxy.id());
            }
            if let zwp_tablet_pad_v2::Event::Group { pad_group } = event {
                state.groups.push(pad_group);
            }
        }

        wayland_client::event_created_child!(State, ZwpTabletPadV2, [
            zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (ZwpTabletPadGroupV2, ()),
        ]);
    }

    impl Dispatch<ZwpTabletPadGroupV2, ()> for State {
        fn event(
            state: &mut Self,
            _: &ZwpTabletPadGroupV2,
            event: zwp_tablet_pad_group_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zwp_tablet_pad_group_v2::Event::Ring { ring } => state.rings.push(ring),
                zwp_tablet_pad_group_v2::Event::Strip { strip } => state.strips.push(strip),
                zwp_tablet_pad_group_v2::Event::Dial { dial } => state.dials.push(dial),
                _ => {}
            }
        }

        wayland_client::event_created_child!(State, ZwpTabletPadGroupV2, [
            zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (ZwpTabletPadRingV2, ()),
            zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (ZwpTabletPadStripV2, ()),
            zwp_tablet_pad_group_v2::EVT_DIAL_OPCODE => (ZwpTabletPadDialV2, ()),
        ]);
    }

    impl Dispatch<ZwpTabletPadRingV2, ()> for State {
        fn event(
            _: &mut Self,
            _: &ZwpTabletPadRingV2,
            _: wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_ring_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ZwpTabletPadStripV2, ()> for State {
        fn event(
            _: &mut Self,
            _: &ZwpTabletPadStripV2,
            _: wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_strip_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ZwpTabletPadDialV2, ()> for State {
        fn event(
            _: &mut Self,
            _: &ZwpTabletPadDialV2,
            _: wayland_protocols::wp::tablet::zv2::client::zwp_tablet_pad_dial_v2::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ZwpTabletToolV2, Tool> for State {
        fn event(
            state: &mut Self,
            proxy: &ZwpTabletToolV2,
            event: zwp_tablet_tool_v2::Event,
            tool: &Tool,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let mut tool = tool.inner.lock().unwrap_or_else(|p| p.into_inner());
            match event {
                zwp_tablet_tool_v2::Event::Type { tool_type } => {
                    tool.eraser = matches!(
                        tool_type,
                        wayland_client::WEnum::Value(zwp_tablet_tool_v2::Type::Eraser)
                    );
                }
                zwp_tablet_tool_v2::Event::ProximityIn { surface, .. } => {
                    tool.focused = surface == state.surface;
                    tool.contact = false;
                    tool.phase = Phase::Move;
                }
                zwp_tablet_tool_v2::Event::ProximityOut => {
                    push_sample(&state.samples, canceled_sample());
                    tool.focused = false;
                    tool.contact = false;
                    tool.barrel_buttons.clear();
                    tool.pressure = None;
                    tool.tilt = [None; 2];
                }
                zwp_tablet_tool_v2::Event::Motion { x, y } => tool.position = Some((x, y)),
                zwp_tablet_tool_v2::Event::Pressure { pressure } => {
                    tool.pressure = Some(wayland_pressure(pressure))
                }
                zwp_tablet_tool_v2::Event::Tilt { tilt_x, tilt_y } => {
                    tool.tilt = [Some(wayland_tilt(tilt_x)), Some(wayland_tilt(tilt_y))]
                }
                zwp_tablet_tool_v2::Event::Down { .. } => {
                    tool.contact = true;
                    tool.phase = Phase::Down;
                }
                zwp_tablet_tool_v2::Event::Up => {
                    tool.contact = false;
                    tool.phase = Phase::Up;
                }
                zwp_tablet_tool_v2::Event::Button {
                    button,
                    state: button_state,
                    ..
                } => {
                    if !matches!(button, 0x14b | 0x14c) {
                        return;
                    }
                    if matches!(
                        button_state,
                        wayland_client::WEnum::Value(zwp_tablet_tool_v2::ButtonState::Pressed)
                    ) {
                        tool.barrel_buttons.insert(button);
                    } else {
                        tool.barrel_buttons.remove(&button);
                    }
                }
                zwp_tablet_tool_v2::Event::Frame { .. } => {
                    if tool.focused {
                        if let Some((x, y)) = tool.position {
                            push_sample(
                                &state.samples,
                                PenSample {
                                    position: wayland_position(x, y, state.scale),
                                    pressure: tool.pressure,
                                    tilt: tool.tilt,
                                    eraser: tool.eraser,
                                    barrel: !tool.barrel_buttons.is_empty(),
                                    contact: tool.contact,
                                    phase: tool.phase,
                                },
                            );
                            tool.phase = Phase::Move;
                        }
                    }
                }
                zwp_tablet_tool_v2::Event::Removed => {
                    if tool.contact {
                        push_sample(&state.samples, canceled_sample());
                    }
                    proxy.destroy();
                    state.tools.retain(|item| item.id() != proxy.id());
                }
                _ => {}
            }
            state.context.request_repaint();
        }
    }

    fn push_sample(samples: &Mutex<Vec<PenSample>>, sample: PenSample) {
        let mut samples = samples.lock().unwrap_or_else(|p| p.into_inner());
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
            tilt: [None; 2],
            eraser: false,
            barrel: false,
            contact: false,
            phase: Phase::Cancel,
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_input {
    use super::*;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub enum PenInput {
        X11(super::linux_x11::X11PenInput),
        Wayland(Box<super::linux_wayland::PenInput>),
    }

    impl PenInput {
        pub fn install(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
            let handle = cc.window_handle().map_err(|e| e.to_string())?;
            match handle.as_raw() {
                RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_) => {
                    super::linux_x11::X11PenInput::install(cc).map(Self::X11)
                }
                RawWindowHandle::Wayland(_) => super::linux_wayland::PenInput::install(cc)
                    .map(Box::new)
                    .map(Self::Wayland),
                _ => Err("Native Linux pen input requires an X11 or Wayland window.".into()),
            }
        }

        pub fn drain(&mut self) -> Vec<PenSample> {
            match self {
                Self::X11(pen) => pen.drain(),
                Self::Wayland(pen) => pen.drain(),
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux_input::PenInput;

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
            let backing_scale = window.backingScaleFactor() as f32;
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
                    ((point.x - bounds.origin.x) as f32) * backing_scale,
                    ((bounds.origin.y + bounds.size.height - point.y) as f32) * backing_scale,
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
