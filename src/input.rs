use eframe::egui;

#[cfg_attr(not(windows), allow(dead_code))]
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

#[cfg(not(windows))]
pub struct PenInput;
#[cfg(not(windows))]
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
