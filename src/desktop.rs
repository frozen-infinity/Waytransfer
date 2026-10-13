//! Linux Wayland output capture and virtual input. Does not own or reconfigure the compositor.
use crate::{
    desktop_frame::{CaptureLayout, PixelFormat},
    Dimensions, Message,
};
use memmap2::{MmapMut, MmapOptions};
use std::{
    collections::BTreeSet,
    error::Error,
    fs::File,
    io::{self, Write},
    os::fd::{AsFd, AsRawFd},
    time::{Duration, Instant},
};
use wayland_client::{
    delegate_noop,
    protocol::{wl_buffer, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool},
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1 as keyboard_manager, zwp_virtual_keyboard_v1 as keyboard,
};
use wayland_protocols_wlr::{
    screencopy::v1::client::{
        zwlr_screencopy_frame_v1 as frame, zwlr_screencopy_manager_v1 as capture,
    },
    virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1 as pointer_manager, zwlr_virtual_pointer_v1 as pointer,
    },
};
use xkbcommon::xkb;
#[derive(Clone)]
pub struct OutputInfo {
    pub global: u32,
    pub output: wl_output::WlOutput,
    pub name: String,
    pub width: i32,
    pub height: i32,
}
struct CopyBuffer {
    layout: CaptureLayout,
    _file: File,
    map: MmapMut,
    buffer: wl_buffer::WlBuffer,
}
#[derive(Default)]
pub struct DesktopState {
    pub outputs: Vec<OutputInfo>,
    selected: Option<u32>,
    shm: Option<wl_shm::WlShm>,
    capture: Option<capture::ZwlrScreencopyManagerV1>,
    pointer_manager: Option<pointer_manager::ZwlrVirtualPointerManagerV1>,
    keyboard_manager: Option<keyboard_manager::ZwpVirtualKeyboardManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<pointer::ZwlrVirtualPointerV1>,
    keyboard: Option<keyboard::ZwpVirtualKeyboardV1>,
    keymap_file: Option<File>,
    key_state: Option<xkb::State>,
    held: BTreeSet<u16>,
    buttons: [bool; 3],
    active: bool,
    copy: Option<CopyBuffer>,
    ready: bool,
    pub pending: bool,
    pub error: Option<String>,
}
impl DesktopState {
    pub fn select(
        &mut self,
        name: Option<&str>,
        view_only: bool,
        layout: &str,
        qh: &QueueHandle<Self>,
    ) -> Result<String, Box<dyn Error>> {
        if layout.contains('\0') {
            return Err("invalid keyboard layout".into());
        }
        let selected = match name {
            Some(name) => self
                .outputs
                .iter()
                .find(|o| o.name == name)
                .ok_or("output not found; use --list-outputs")?,
            None if self.outputs.len() == 1 => &self.outputs[0],
            None if self.outputs.is_empty() => return Err("No Wayland outputs available".into()),
            None => return Err("Multiple outputs found; choose --output NAME".into()),
        }
        .clone();
        self.shm.as_ref().ok_or("wl_shm is unavailable")?;
        self.capture
            .as_ref()
            .ok_or("wlr-screencopy is unavailable or denied; check compositor permissions")?;
        self.selected = Some(selected.global);
        if !view_only {
            let seat = self.seat.as_ref().ok_or("No Wayland seat available")?;
            let manager = self.pointer_manager.as_ref().ok_or(
                "virtual-pointer unavailable; use --view-only or enable its compositor permission",
            )?;
            if manager.version() < 2 {
                return Err("Output-bound virtual pointer v2 required; use --view-only".into());
            }
            self.pointer = Some(manager.create_virtual_pointer_with_output(
                Some(seat),
                Some(&selected.output),
                qh,
                (),
            ));
            let manager = self.keyboard_manager.as_ref().ok_or(
                "virtual-keyboard unavailable; use --view-only or enable its compositor permission",
            )?;
            let keyboard = manager.create_virtual_keyboard(seat, qh, ());
            let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
            let keymap = xkb::Keymap::new_from_names(
                &context,
                "",
                "",
                layout,
                "",
                None,
                xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
            .ok_or("could not compile keyboard layout")?;
            let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
            let mut file = tempfile::tempfile()?;
            file.write_all(text.as_bytes())?;
            file.write_all(&[0])?;
            keyboard.keymap(1, file.as_fd(), (text.len() + 1) as u32);
            self.key_state = Some(xkb::State::new(&keymap));
            self.keymap_file = Some(file);
            self.keyboard = Some(keyboard);
        }
        Ok(selected.name)
    }
    pub fn request_capture(&mut self, qh: &QueueHandle<Self>) {
        if self.pending {
            return;
        }
        let Some(output) = self
            .outputs
            .iter()
            .find(|o| Some(o.global) == self.selected)
        else {
            self.error = Some("Selected output disappeared".into());
            return;
        };
        self.capture
            .as_ref()
            .unwrap()
            .capture_output(1, &output.output, qh, ());
        self.pending = true;
    }
    fn copy_to_buffer(&mut self, frame: &frame::ZwlrScreencopyFrameV1) {
        if let Some(copy) = &self.copy {
            frame.copy(&copy.buffer);
        } else {
            self.error = Some("Output has no supported SHM capture buffer".into());
        }
    }
    pub fn take_frame(&mut self, bound: Dimensions) -> io::Result<Option<(Dimensions, Vec<u32>)>> {
        if !std::mem::take(&mut self.ready) {
            return Ok(None);
        }
        let copy = self
            .copy
            .take()
            .ok_or_else(|| io::Error::other("ready without a capture buffer"))?;
        let result = copy.layout.convert(&copy.map, bound);
        copy.buffer.destroy();
        result.map(Some)
    }
    pub fn input(&mut self, message: Message, size: Dimensions, time: u32) {
        match message {
            Message::Focus { active } => {
                if !active {
                    self.release(time);
                }
                self.active = active;
            }
            Message::Keys { held } if self.active => {
                let next: BTreeSet<_> = held.into_iter().collect();
                let released: Vec<_> = self.held.difference(&next).copied().collect();
                let mut pressed: Vec<_> = next.difference(&self.held).copied().collect();
                pressed
                    .sort_by_key(|k| (!matches!(k, 29 | 42 | 54 | 56 | 97 | 100 | 125 | 126), *k));
                for key in released {
                    self.key(key, false, time);
                }
                for key in pressed {
                    self.key(key, true, time);
                }
                self.held = next;
            }
            Message::Pointer { x, y, pressed } if self.active => {
                if let Some(pointer) = &self.pointer {
                    pointer.motion_absolute(
                        time,
                        u32::from(x.min(size.width - 1)),
                        u32::from(y.min(size.height - 1)),
                        u32::from(size.width),
                        u32::from(size.height),
                    );
                }
                self.button(0, pressed, time);
                self.pointer_frame();
            }
            Message::Buttons { right, middle } if self.active => {
                self.button(1, right, time);
                self.button(2, middle, time);
                self.pointer_frame();
            }
            Message::Scroll {
                horizontal,
                vertical,
            } if self.active => {
                if let Some(pointer) = &self.pointer {
                    pointer.axis_source(wl_pointer::AxisSource::Wheel);
                    if horizontal != 0 {
                        pointer.axis(
                            time,
                            wl_pointer::Axis::HorizontalScroll,
                            f64::from(horizontal),
                        );
                    }
                    if vertical != 0 {
                        pointer.axis(time, wl_pointer::Axis::VerticalScroll, f64::from(vertical));
                    }
                    pointer.frame();
                }
            }
            _ => {}
        }
    }
    fn key(&mut self, key: u16, pressed: bool, time: u32) {
        if let (Some(keyboard), Some(state)) = (&self.keyboard, &mut self.key_state) {
            keyboard.key(time, u32::from(key), u32::from(pressed));
            state.update_key(
                (u32::from(key) + 8).into(),
                if pressed {
                    xkb::KeyDirection::Down
                } else {
                    xkb::KeyDirection::Up
                },
            );
            keyboard.modifiers(
                state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
                state.serialize_mods(xkb::STATE_MODS_LATCHED),
                state.serialize_mods(xkb::STATE_MODS_LOCKED),
                state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
            );
        }
    }
    fn button(&mut self, index: usize, pressed: bool, time: u32) {
        if self.buttons[index] != pressed {
            if let Some(pointer) = &self.pointer {
                pointer.button(
                    time,
                    0x110 + index as u32,
                    if pressed {
                        wl_pointer::ButtonState::Pressed
                    } else {
                        wl_pointer::ButtonState::Released
                    },
                );
            }
            self.buttons[index] = pressed;
        }
    }
    fn pointer_frame(&self) {
        if let Some(pointer) = &self.pointer {
            pointer.frame();
        }
    }
    pub fn release(&mut self, time: u32) {
        for key in std::mem::take(&mut self.held) {
            self.key(key, false, time);
        }
        for index in 0..3 {
            self.button(index, false, time);
        }
        self.pointer_frame();
        self.active = false;
    }
    pub fn destroy_input(&mut self) {
        if let Some(keyboard) = self.keyboard.take() {
            keyboard.destroy();
        }
        if let Some(pointer) = self.pointer.take() {
            pointer.destroy();
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for DesktopState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_output" => {
                    let output = registry.bind(name, version.min(4), qh, name);
                    state.outputs.push(OutputInfo {
                        global: name,
                        output,
                        name: format!("output-{name}"),
                        width: 0,
                        height: 0,
                    });
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(7), qh, ()))
                }
                "zwlr_screencopy_manager_v1" => {
                    state.capture = Some(registry.bind(name, version.min(3), qh, ()))
                }
                "zwlr_virtual_pointer_manager_v1" => {
                    state.pointer_manager = Some(registry.bind(name, version.min(2), qh, ()))
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.keyboard_manager = Some(registry.bind(name, 1, qh, ()))
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.retain(|o| o.global != name);
                if state.selected == Some(name) {
                    state.error = Some(
                        "Captured output removed; reconnect after choosing a new output".into(),
                    );
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_output::WlOutput, u32> for DesktopState {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Some(output) = state.outputs.iter_mut().find(|o| o.global == *id) {
            match event {
                wl_output::Event::Name { name } => output.name = name,
                wl_output::Event::Mode {
                    flags: WEnum::Value(flags),
                    width,
                    height,
                    ..
                } if flags.contains(wl_output::Mode::Current) => {
                    output.width = width;
                    output.height = height;
                }
                _ => {}
            }
        }
    }
}
impl Dispatch<frame::ZwlrScreencopyFrameV1, ()> for DesktopState {
    fn event(
        state: &mut Self,
        frame: &frame::ZwlrScreencopyFrameV1,
        event: frame::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            frame::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                let result = (|| -> Result<CopyBuffer, Box<dyn Error>> {
                    let pixel_format = match format {
                        wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => PixelFormat::Rgb,
                        wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => PixelFormat::Bgr,
                        wl_shm::Format::Argb2101010 | wl_shm::Format::Xrgb2101010 => {
                            PixelFormat::Rgb10
                        }
                        wl_shm::Format::Abgr2101010 | wl_shm::Format::Xbgr2101010 => {
                            PixelFormat::Bgr10
                        }
                        _ => {
                            return Err(
                                format!("Unsupported screencopy SHM format {format:?}").into()
                            )
                        }
                    };
                    let layout = CaptureLayout {
                        width: width as usize,
                        height: height as usize,
                        stride: stride as usize,
                        format: pixel_format,
                        inverted: false,
                    };
                    let len = layout.byte_len()?;
                    let file = tempfile::tempfile()?;
                    file.set_len(len as u64)?;
                    // SAFETY: private fixed-size SHM file is retained with the mapping. We read
                    // only after frame Ready; this file is never resized or reused for capture.
                    let map = unsafe { MmapOptions::new().len(len).map_mut(&file)? };
                    let pool =
                        state
                            .shm
                            .as_ref()
                            .unwrap()
                            .create_pool(file.as_fd(), len as i32, qh, ());
                    let buffer = pool.create_buffer(
                        0,
                        width as i32,
                        height as i32,
                        stride as i32,
                        format,
                        qh,
                        (),
                    );
                    pool.destroy();
                    Ok(CopyBuffer {
                        layout,
                        _file: file,
                        map,
                        buffer,
                    })
                })();
                match result {
                    Ok(copy) => state.copy = Some(copy),
                    Err(error) => state.error = Some(error.to_string()),
                }
                if frame.version() < 3 && state.error.is_none() {
                    state.copy_to_buffer(frame);
                }
            }
            frame::Event::BufferDone => {
                if state.error.is_none() {
                    state.copy_to_buffer(frame);
                }
            }
            frame::Event::Flags {
                flags: WEnum::Value(flags),
            } => {
                if let Some(copy) = &mut state.copy {
                    copy.layout.inverted = flags.contains(frame::Flags::YInvert);
                }
            }
            frame::Event::Ready { .. } => {
                state.ready = true;
                state.pending = false;
                frame.destroy();
            }
            frame::Event::Failed => {
                state.pending = false;
                state.error = Some(
                    "Screencopy failed: check output and Hyprland screencopy permission".into(),
                );
                frame.destroy();
            }
            _ => {}
        }
    }
}
delegate_noop!(DesktopState:ignore wl_shm::WlShm);
delegate_noop!(DesktopState:ignore wl_shm_pool::WlShmPool);
delegate_noop!(DesktopState:ignore wl_buffer::WlBuffer);
delegate_noop!(DesktopState:ignore wl_seat::WlSeat);
delegate_noop!(DesktopState:ignore capture::ZwlrScreencopyManagerV1);
delegate_noop!(DesktopState:ignore pointer_manager::ZwlrVirtualPointerManagerV1);
delegate_noop!(DesktopState:ignore pointer::ZwlrVirtualPointerV1);
delegate_noop!(DesktopState:ignore keyboard_manager::ZwpVirtualKeyboardManagerV1);
delegate_noop!(DesktopState:ignore keyboard::ZwpVirtualKeyboardV1);
/// Pump Wayland while leaving network input/Ctrl-C responsive on an idle desktop.
pub fn pump(
    connection: &Connection,
    queue: &mut wayland_client::EventQueue<DesktopState>,
    state: &mut DesktopState,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    queue.dispatch_pending(state)?;
    connection.flush()?;
    if let Some(guard) = queue.prepare_read() {
        let mut fd = libc::pollfd {
            fd: connection.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd lives for this call; the borrowed connection fd
        // remains open and is not mutated or closed from another thread.
        let ready =
            unsafe { libc::poll(&mut fd, 1, timeout.as_millis().min(i32::MAX as u128) as i32) };
        if ready > 0 {
            guard.read()?;
        } else if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(io::Error::last_os_error().into());
        }
    }
    queue.dispatch_pending(state)?;
    Ok(())
}
pub fn time(start: Instant) -> u32 {
    start.elapsed().as_millis() as u32
}
