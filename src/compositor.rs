//! Headless multiwindow Wayland session with optional EGL and XWayland backends.
use crate::{
    remote::WindowSnapshot, render_engine::RenderEngine, Dimensions, Message, MAX_SCENE_PIXELS,
    MAX_WINDOWS,
};
use smithay::{
    backend::{
        input::{Axis, AxisSource, ButtonState, KeyState},
        renderer::{
            element::{surface::WaylandSurfaceRenderElement, AsRenderElements},
            pixman::PixmanRenderer,
            utils::{draw_render_elements, on_commit_buffer_handler},
            Bind, Color32F, ExportMem, Frame, Offscreen, Renderer,
        },
    },
    desktop::{
        space::SpaceElement, PopupKeyboardGrab, PopupManager, PopupPointerGrab, Window,
        WindowSurfaceType,
    },
    input::{
        keyboard::{FilterResult, XkbConfig},
        pointer::{AxisFrame, ButtonEvent, CursorImageStatus, Focus, MotionEvent},
        Seat, SeatHandler, SeatState,
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        pixman,
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1, shell::server::xdg_toplevel,
        },
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer, wl_seat, wl_surface::WlSurface},
            Client, Display, DisplayHandle, Resource,
        },
    },
    utils::{Rectangle, Serial, Transform, SERIAL_COUNTER},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            with_states, CompositorClientState, CompositorHandler, CompositorState,
            SurfaceAttributes,
        },
        fractional_scale::{self, FractionalScaleHandler, FractionalScaleManagerState},
        output::{OutputHandler, OutputManagerState},
        selection::{
            data_device::{
                set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
                ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{
            decoration::{XdgDecorationHandler, XdgDecorationState},
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
        viewporter::ViewporterState,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    os::unix::io::OwnedFd,
    sync::Arc,
    time::Instant,
};

#[derive(Default)]
pub struct ClientState {
    compositor_state: CompositorClientState,
}
impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

struct WindowEntry {
    window: Window,
    size: Dimensions,
    pixels: Option<Arc<Vec<u32>>>,
    dirty: bool,
}
pub struct SingleWindow {
    windows: BTreeMap<u32, WindowEntry>,
    next_id: u32,
    selected: Option<u32>,
    scene_dirty: bool,
    #[cfg(feature = "x11")]
    pub(crate) xwm: Option<smithay::xwayland::X11Wm>,
    #[cfg(feature = "x11")]
    pub(crate) xwayland_shell: smithay::wayland::xwayland_shell::XWaylandShellState,
    #[cfg(feature = "x11")]
    pub x11_display: Option<u32>,
    #[cfg(feature = "gpu")]
    pub(crate) dmabuf_state: smithay::wayland::dmabuf::DmabufState,
    #[cfg(feature = "gpu")]
    pub(crate) import_renderer: Option<smithay::backend::renderer::gles::GlesRenderer>,

    display: DisplayHandle,
    compositor: CompositorState,
    shell: XdgShellState,
    shm: ShmState,
    seats: SeatState<Self>,
    data_device: DataDeviceState,
    _output_manager: OutputManagerState,
    _decorations: XdgDecorationState,
    _viewporter: ViewporterState,
    _fractional_scale: FractionalScaleManagerState,
    pub seat: Seat<Self>,
    pub output: Output,
    window: Option<Window>,
    size: Dimensions,
    initial_size: Dimensions,
    popups: PopupManager,
    dirty: bool,
    start: Instant,
    held: BTreeSet<u16>,
    buttons: [bool; 3],
    active: bool,
}
impl SingleWindow {
    pub fn new(display: &DisplayHandle) -> Result<Self, Box<dyn Error>> {
        Self::new_sized(display, Dimensions::default())
    }
    pub fn new_sized(display: &DisplayHandle, size: Dimensions) -> Result<Self, Box<dyn Error>> {
        let size = size.validate()?;
        let mut seats = SeatState::new();
        let mut seat = seats.new_wl_seat(display, "waytransfer");
        seat.add_keyboard(
            XkbConfig {
                layout: "us",
                ..Default::default()
            },
            500,
            25,
        )?;
        seat.add_pointer();
        let output = Output::new(
            "Waytransfer".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Waytransfer".into(),
                model: "Single window".into(),
            },
        );
        output.create_global::<Self>(display);
        let mode = Mode {
            size: (size.width as i32, size.height as i32).into(),
            refresh: 30000,
        };
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            Some((0, 0).into()),
        );
        output.set_preferred(mode);
        Ok(Self {
            windows: BTreeMap::new(),
            next_id: 1,
            selected: None,
            scene_dirty: true,
            #[cfg(feature = "x11")]
            xwm: None,
            #[cfg(feature = "x11")]
            xwayland_shell: smithay::wayland::xwayland_shell::XWaylandShellState::new::<Self>(
                display,
            ),
            #[cfg(feature = "x11")]
            x11_display: None,
            #[cfg(feature = "gpu")]
            dmabuf_state: smithay::wayland::dmabuf::DmabufState::new(),
            #[cfg(feature = "gpu")]
            import_renderer: None,
            display: display.clone(),
            compositor: CompositorState::new::<Self>(display),
            shell: XdgShellState::new::<Self>(display),
            shm: ShmState::new::<Self>(display, vec![]),
            data_device: DataDeviceState::new::<Self>(display),
            _output_manager: OutputManagerState::new_with_xdg_output::<Self>(display),
            _decorations: XdgDecorationState::new::<Self>(display),
            _viewporter: ViewporterState::new::<Self>(display),
            _fractional_scale: FractionalScaleManagerState::new::<Self>(display),
            seats,
            seat,
            output,
            window: None,
            size,
            initial_size: size,
            popups: PopupManager::default(),
            dirty: true,
            start: Instant::now(),
            held: BTreeSet::new(),
            buttons: [false; 3],
            active: true,
        })
    }
    pub fn insert_client(
        &self,
        display: &DisplayHandle,
        stream: std::os::unix::net::UnixStream,
    ) -> std::io::Result<Client> {
        display
            .clone()
            .insert_client(stream, Arc::new(ClientState::default()))
    }
    #[cfg(feature = "x11")]
    pub(crate) fn window_entries(&self) -> impl Iterator<Item = (u32, &Window)> {
        self.windows.iter().map(|(id, entry)| (*id, &entry.window))
    }
    #[cfg(feature = "x11")]
    pub(crate) fn mark_all_dirty(&mut self) {
        for entry in self.windows.values_mut() {
            entry.dirty = true;
        }
        self.scene_dirty = true;
    }
    #[cfg(feature = "x11")]
    pub(crate) fn mark_metadata_dirty(&mut self) {
        self.scene_dirty = true;
    }
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }
    pub(crate) fn register_window(&mut self, window: Window, size: Dimensions) -> Option<u32> {
        if self.windows.len() >= MAX_WINDOWS
            || self
                .windows
                .values()
                .map(|w| w.size.pixel_count())
                .sum::<usize>()
                + size.pixel_count()
                > MAX_SCENE_PIXELS
        {
            eprintln!("Window resource budget reached");
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)?;
        self.windows.insert(
            id,
            WindowEntry {
                window: window.clone(),
                size,
                pixels: None,
                dirty: true,
            },
        );
        self.scene_dirty = true;
        if self.selected.is_none() {
            self.selected = Some(id);
            self.window = Some(window);
            self.size = size;
            self.keyboard_focus();
        }
        Some(id)
    }
    pub(crate) fn remove_window(&mut self, id: u32) {
        if self.selected == Some(id) {
            self.release_input();
            self.selected = None;
            self.window = None;
        }
        self.windows.remove(&id);
        self.scene_dirty = true;
        if self.selected.is_none() {
            if let Some((&next, entry)) = self.windows.iter().next() {
                self.selected = Some(next);
                self.window = Some(entry.window.clone());
                self.size = entry.size;
            }
            self.keyboard_focus();
        }
    }
    pub fn input_window(&mut self, id: u32, message: Message) -> Result<(), &'static str> {
        if !self.windows.contains_key(&id) {
            return Ok(());
        }
        match message {
            Message::Close => {
                let window = &self.windows[&id].window;
                if let Some(top) = window.toplevel() {
                    top.send_close();
                }
                #[cfg(feature = "x11")]
                if let Some(surface) = window.x11_surface() {
                    let _ = surface.close();
                }
                Ok(())
            }
            Message::Resize { size } => {
                size.validate().map_err(|_| "invalid size")?;
                let total: usize = self
                    .windows
                    .iter()
                    .filter(|(other, _)| **other != id)
                    .map(|(_, w)| w.size.pixel_count())
                    .sum();
                if total + size.pixel_count() > MAX_SCENE_PIXELS {
                    return Err("scene pixel budget");
                }
                let selected = self.selected;
                let old_window = self.window.clone();
                let old_size = self.size;
                self.window = Some(self.windows[&id].window.clone());
                self.size = self.windows[&id].size;
                self.resize(size).map_err(|_| "invalid size")?;
                let entry = self.windows.get_mut(&id).unwrap();
                entry.size = size;
                entry.dirty = true;
                self.scene_dirty = true;
                #[cfg(feature = "x11")]
                if let Some(surface) = entry.window.x11_surface() {
                    let _ = surface.configure(Rectangle::from_size(
                        (size.width as i32, size.height as i32).into(),
                    ));
                }
                if selected != Some(id) {
                    self.window = old_window;
                    self.size = old_size;
                }
                Ok(())
            }
            Message::Focus { active: true } => {
                if self.selected != Some(id) {
                    self.release_input();
                    let keyboard = self.seat.get_keyboard().unwrap();
                    keyboard.unset_grab(self);
                    self.seat.get_pointer().unwrap().unset_grab(
                        self,
                        SERIAL_COUNTER.next_serial(),
                        self.start.elapsed().as_millis() as u32,
                    );
                    self.selected = Some(id);
                    self.window = Some(self.windows[&id].window.clone());
                    self.size = self.windows[&id].size;
                }
                for (other, entry) in &self.windows {
                    entry.window.set_activated(*other == id);
                    if let Some(top) = entry.window.toplevel() {
                        top.send_pending_configure();
                    }
                }
                self.scene_dirty = true;
                self.input(Message::Focus { active: true })
            }
            other if self.selected == Some(id) => self.input(other),
            _ => Ok(()),
        }
    }
    pub fn render_windows(
        &mut self,
        renderer: &mut RenderEngine,
    ) -> Result<Option<Vec<WindowSnapshot>>, Box<dyn Error>> {
        self.keyboard_focus();
        self.popups.cleanup();
        let mut changed = std::mem::take(&mut self.scene_dirty);
        for entry in self.windows.values_mut() {
            if entry.dirty {
                entry.dirty = false;
                entry.window.on_commit();
                if entry.window.bbox().size.w > 0 && entry.window.bbox().size.h > 0 {
                    entry.pixels =
                        Some(Arc::new(renderer.render(Some(&entry.window), entry.size)?));
                    entry.window.with_surfaces(|_, states| {
                        for callback in states
                            .cached_state
                            .get::<SurfaceAttributes>()
                            .current()
                            .frame_callbacks
                            .drain(..)
                        {
                            callback.done(self.start.elapsed().as_millis() as u32);
                        }
                    });
                } else {
                    entry.pixels = None;
                }
                changed = true;
            }
        }
        if !changed {
            return Ok(None);
        }
        let mut snapshots = Vec::new();
        for (&id, entry) in &self.windows {
            if let Some(pixels) = &entry.pixels {
                let mut parent = None;
                let mut title = String::new();
                if let Some(top) = entry.window.toplevel() {
                    with_states(top.wl_surface(), |states| {
                        let data = states
                            .data_map
                            .get::<XdgToplevelSurfaceData>()
                            .unwrap()
                            .lock()
                            .unwrap();
                        title = data.title.clone().unwrap_or_default();
                        parent = data.parent.as_ref().and_then(|surface| {
                            self.windows
                                .iter()
                                .find(|(_, entry)| {
                                    entry
                                        .window
                                        .toplevel()
                                        .is_some_and(|top| top.wl_surface() == surface)
                                })
                                .map(|(id, _)| *id)
                        });
                    });
                }
                #[cfg(feature = "x11")]
                if let Some(surface) = entry.window.x11_surface() {
                    title = surface.title();
                    parent = surface.is_transient_for().and_then(|parent| {
                        self.windows
                            .iter()
                            .find(|(_, entry)| {
                                entry
                                    .window
                                    .x11_surface()
                                    .is_some_and(|surface| surface.window_id() == parent)
                            })
                            .map(|(id, _)| *id)
                    });
                }
                snapshots.push(WindowSnapshot {
                    id,
                    parent,
                    title: crate::window_title(&title),
                    size: entry.size,
                    pixels: pixels.clone(),
                });
            }
        }
        Ok(Some(snapshots))
    }
    pub fn dimensions(&self) -> Dimensions {
        self.size
    }
    pub fn resize(&mut self, size: Dimensions) -> std::io::Result<()> {
        let size = size.validate()?;
        if self.size == size {
            return Ok(());
        }
        self.size = size;
        let previous = self.output.current_mode();
        let mode = Mode {
            size: (size.width as i32, size.height as i32).into(),
            refresh: 30000,
        };
        self.output
            .change_current_state(Some(mode), None, None, None);
        self.output.set_preferred(mode);
        if let Some(previous) = previous {
            self.output.delete_mode(previous);
        }
        if let Some(window) = &self.window {
            if let Some(top) = window.toplevel() {
                top.with_pending_state(|state| {
                    state.size = Some((size.width as i32, size.height as i32).into())
                });
                top.send_configure();
            }
        }
        self.dirty = true;
        Ok(())
    }
    fn root(&self) -> Option<WlSurface> {
        let window = self.window.as_ref()?;
        if let Some(top) = window.toplevel() {
            return Some(top.wl_surface().clone());
        }
        #[cfg(feature = "x11")]
        if let Some(surface) = window.x11_surface() {
            return surface.wl_surface();
        }
        None
    }
    fn focus_target(&self) -> Option<crate::keyboard_focus::KeyboardFocus> {
        #[cfg(feature = "x11")]
        let window = self.window.as_ref()?;
        #[cfg(feature = "x11")]
        if let Some(surface) = window.x11_surface() {
            surface.wl_surface()?;
            return Some(crate::keyboard_focus::KeyboardFocus::X11(surface.clone()));
        }
        self.root().map(Into::into)
    }
    fn keyboard_focus(&mut self) {
        if self.seat.get_keyboard().unwrap().is_grabbed() {
            return;
        }
        let surface = if self.active {
            self.focus_target()
        } else {
            None
        };
        let keyboard = self.seat.get_keyboard().unwrap();
        if keyboard.current_focus() != surface {
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
    }
    /// Called on viewer disconnect/focus loss so held modifiers and mouse grabs cannot stick.
    pub fn release_input(&mut self) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let time = self.start.elapsed().as_millis() as u32;
        let held = std::mem::take(&mut self.held);
        for key in held {
            keyboard.input::<(), _>(
                self,
                (u32::from(key) + 8).into(),
                KeyState::Released,
                SERIAL_COUNTER.next_serial(),
                time,
                |_, _, _| FilterResult::Forward,
            );
        }
        let pointer = self.seat.get_pointer().unwrap();
        for i in 0..3 {
            if self.buttons[i] {
                pointer.button(
                    self,
                    &ButtonEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                        button: 0x110 + i as u32,
                        state: ButtonState::Released,
                    },
                );
                self.buttons[i] = false;
            }
        }
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location: pointer.current_location(),
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }
    pub fn input(&mut self, message: Message) -> Result<(), &'static str> {
        let time = self.start.elapsed().as_millis() as u32;
        self.keyboard_focus();
        match message {
            Message::ForWindow { id, message } => return self.input_window(id, *message),
            Message::Resize { size } => {
                self.resize(size).map_err(|_| "unsupported resize")?;
            }
            Message::Focus { active } => {
                if !active {
                    self.release_input();
                }
                self.active = active;
                self.keyboard_focus();
            }
            Message::Pointer { x, y, pressed } => {
                // Requests and frame updates cross in flight during a resize.
                let x = x.min(self.size.width - 1);
                let y = y.min(self.size.height - 1);
                if !self.active {
                    return Ok(());
                }
                let point = (f64::from(x), f64::from(y)).into();
                // Window geometry may include client-side shadow margins. Match rendering origin.
                let origin = self
                    .window
                    .as_ref()
                    .map(|w| w.geometry().loc)
                    .unwrap_or_default();
                let focus = self
                    .window
                    .as_ref()
                    .and_then(|w| w.surface_under(point + origin.to_f64(), WindowSurfaceType::ALL))
                    .map(|(s, p)| (s, (p - origin).to_f64()));
                let pointer = self.seat.get_pointer().unwrap();
                pointer.motion(
                    self,
                    focus,
                    &MotionEvent {
                        location: point,
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
                self.button(0, pressed, time);
                pointer.frame(self);
            }
            Message::Buttons { right, middle } => {
                if self.active {
                    self.button(1, right, time);
                    self.button(2, middle, time);
                    self.seat.get_pointer().unwrap().frame(self);
                }
            }
            Message::Keys { held } => {
                if held.len() > 128 || held.iter().any(|&key| key == 0 || key > 127) {
                    return Err("unsupported key snapshot");
                }
                if !self.active {
                    return Ok(());
                }
                let next: BTreeSet<_> = held.into_iter().collect();
                let keyboard = self.seat.get_keyboard().unwrap();
                // Releases first; modifier presses precede letters (snapshots have no event order).
                let released: Vec<_> = self.held.difference(&next).copied().collect();
                let mut pressed: Vec<_> = next.difference(&self.held).copied().collect();
                pressed
                    .sort_by_key(|k| (!matches!(k, 29 | 42 | 54 | 56 | 97 | 100 | 125 | 126), *k));
                for key in released {
                    keyboard.input::<(), _>(
                        self,
                        (u32::from(key) + 8).into(),
                        KeyState::Released,
                        SERIAL_COUNTER.next_serial(),
                        time,
                        |_, _, _| FilterResult::Forward,
                    );
                }
                for key in pressed {
                    keyboard.input::<(), _>(
                        self,
                        (u32::from(key) + 8).into(),
                        KeyState::Pressed,
                        SERIAL_COUNTER.next_serial(),
                        time,
                        |_, _, _| FilterResult::Forward,
                    );
                }
                self.held = next;
            }
            Message::Scroll {
                horizontal,
                vertical,
            } => {
                if self.active {
                    let pointer = self.seat.get_pointer().unwrap();
                    pointer.axis(
                        self,
                        AxisFrame::new(time)
                            .source(AxisSource::Wheel)
                            .value(Axis::Horizontal, f64::from(horizontal))
                            .value(Axis::Vertical, f64::from(vertical)),
                    );
                    pointer.frame(self);
                }
            }
            _ => return Err("unexpected viewer message"),
        }
        Ok(())
    }
    fn button(&mut self, index: usize, pressed: bool, time: u32) {
        if self.buttons[index] != pressed {
            self.seat.get_pointer().unwrap().button(
                self,
                &ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                    button: 0x110 + index as u32,
                    state: if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    },
                },
            );
            self.buttons[index] = pressed;
        }
    }
    /// Render only after a commit/lifecycle change. Callback delivery remains separate from TCP.
    pub fn render(
        &mut self,
        renderer: &mut PixmanRenderer,
        image: &mut pixman::Image<'static, 'static>,
    ) -> Result<Option<Vec<u32>>, Box<dyn Error>> {
        self.popups.cleanup();
        self.keyboard_focus();
        if !std::mem::take(&mut self.dirty) {
            return Ok(None);
        }
        if image.width() != self.size.width as usize || image.height() != self.size.height as usize
        {
            *image = renderer.create_buffer(
                smithay::backend::allocator::Fourcc::Argb8888,
                (self.size.width as i32, self.size.height as i32).into(),
            )?;
        }
        let elements: Vec<WaylandSurfaceRenderElement<PixmanRenderer>> = self
            .window
            .as_ref()
            .map(|w| {
                w.render_elements(
                    renderer,
                    (-w.geometry().loc.x, -w.geometry().loc.y).into(),
                    1.0.into(),
                    1.0,
                )
            })
            .unwrap_or_default();
        let mut target = renderer.bind(image)?;
        let damage = Rectangle::from_size((self.size.width as i32, self.size.height as i32).into());
        let mut frame = renderer.render(
            &mut target,
            (self.size.width as i32, self.size.height as i32).into(),
            Transform::Normal,
        )?;
        frame.clear(Color32F::new(0.094, 0.125, 0.173, 1.0), &[damage])?;
        draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
        frame.finish()?.wait()?;
        let mapping = renderer.copy_framebuffer(
            &target,
            Rectangle::from_size((self.size.width as i32, self.size.height as i32).into()),
            smithay::backend::allocator::Fourcc::Argb8888,
        )?;
        let bytes = renderer.map_texture(&mapping)?;
        let pixels = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| u32::from_ne_bytes(*p) & 0xffffff)
            .collect();
        if let Some(window) = &self.window {
            window.with_surfaces(|_, states| {
                for callback in states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .drain(..)
                {
                    callback.done(self.start.elapsed().as_millis() as u32);
                }
            });
        }
        Ok(Some(pixels))
    }
    pub fn new_display() -> Result<Display<Self>, Box<dyn Error>> {
        Ok(Display::new()?)
    }
    pub fn renderer() -> Result<(PixmanRenderer, pixman::Image<'static, 'static>), Box<dyn Error>> {
        let mut renderer = PixmanRenderer::new()?;
        let image = renderer.create_buffer(
            smithay::backend::allocator::Fourcc::Argb8888,
            (
                Dimensions::default().width as i32,
                Dimensions::default().height as i32,
            )
                .into(),
        )?;
        Ok((renderer, image))
    }
}
impl BufferHandler for SingleWindow {
    fn buffer_destroyed(&mut self, _: &wl_buffer::WlBuffer) {}
}
impl CompositorHandler for SingleWindow {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(data) = client.get_data::<ClientState>() {
            return &data.compositor_state;
        }
        #[cfg(feature = "x11")]
        if let Some(data) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &data.compositor_state;
        }
        panic!("client has no compositor state")
    }
    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);
        if let Some(window) = &self.window {
            window.on_commit();
            window.output_enter(
                &self.output,
                Rectangle::from_size((self.size.width as i32, self.size.height as i32).into()),
            );
        }
        let mut total: usize = self
            .windows
            .values()
            .map(|entry| entry.size.pixel_count())
            .sum();
        for (&id, entry) in &mut self.windows {
            entry.window.on_commit();
            let mut matches = false;
            entry.window.with_surfaces(|candidate, _| {
                if candidate == surface {
                    matches = true;
                }
            });
            if matches {
                entry.dirty = true;
                entry.window.output_enter(
                    &self.output,
                    Rectangle::from_size(
                        (entry.size.width as i32, entry.size.height as i32).into(),
                    ),
                );
                let geometry = entry.window.geometry();
                if geometry.size.w > 0 && geometry.size.h > 0 {
                    if let Ok(actual) =
                        Dimensions::new(geometry.size.w as usize, geometry.size.h as usize)
                    {
                        if total - entry.size.pixel_count() + actual.pixel_count()
                            <= MAX_SCENE_PIXELS
                        {
                            total = total - entry.size.pixel_count() + actual.pixel_count();
                            entry.size = actual;
                            if self.selected == Some(id) {
                                self.size = actual;
                            }
                        }
                    }
                }
            }
        }
        self.dirty = true;
        self.keyboard_focus();
    }
    fn destroyed(&mut self, _: &WlSurface) {
        self.scene_dirty = true;
        self.dirty = true;
    }
}
impl ShmHandler for SingleWindow {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}
impl OutputHandler for SingleWindow {}
impl FractionalScaleHandler for SingleWindow {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // The remote canvas currently uses logical pixels at scale 1. Clients
        // may still crop/scale their buffers through wp_viewporter.
        with_states(&surface, |states| {
            fractional_scale::with_fractional_scale(states, |scale| scale.set_preferred_scale(1.0));
        });
    }
}
smithay::delegate_viewporter!(SingleWindow);
smithay::delegate_fractional_scale!(SingleWindow);
impl SeatHandler for SingleWindow {
    type KeyboardFocus = crate::keyboard_focus::KeyboardFocus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seats
    }
    fn focus_changed(
        &mut self,
        seat: &Seat<Self>,
        focused: Option<&crate::keyboard_focus::KeyboardFocus>,
    ) {
        set_data_device_focus(
            &self.display,
            seat,
            focused.and_then(|s| {
                use smithay::wayland::seat::WaylandFocus;
                s.wl_surface().and_then(|surface| surface.client())
            }),
        );
    }
    fn cursor_image(&mut self, _: &Seat<Self>, _: CursorImageStatus) {}
}
impl SelectionHandler for SingleWindow {
    type SelectionUserData = ();
}
impl DataDeviceHandler for SingleWindow {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device
    }
}
impl ClientDndGrabHandler for SingleWindow {}
impl ServerDndGrabHandler for SingleWindow {
    fn send(&mut self, _: String, _: OwnedFd, _: Seat<Self>) {}
}
impl XdgShellHandler for SingleWindow {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.shell
    }
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| {
            state.size = Some(
                (
                    self.initial_size.width as i32,
                    self.initial_size.height as i32,
                )
                    .into(),
            );
            if self.selected.is_none() {
                state.states.set(xdg_toplevel::State::Activated);
            }
        });
        surface.send_configure();
        let window = Window::new_wayland_window(surface.clone());
        if self.register_window(window, self.initial_size).is_none() {
            surface.send_close();
        }
        self.dirty = true;
    }
    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self
            .windows
            .iter()
            .find(|(_, entry)| {
                entry
                    .window
                    .toplevel()
                    .is_some_and(|top| top.wl_surface() == surface.wl_surface())
            })
            .map(|(id, _)| *id)
        {
            self.remove_window(id);
        }
        self.dirty = true;
    }
    fn title_changed(&mut self, _: ToplevelSurface) {
        self.scene_dirty = true;
    }
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| state.states.set(xdg_toplevel::State::Maximized));
        surface.send_configure();
    }
    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| state.states.unset(xdg_toplevel::State::Maximized));
        surface.send_configure();
    }
    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        surface.with_pending_state(|state| state.states.set(xdg_toplevel::State::Fullscreen));
        surface.send_configure();
    }
    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| state.states.unset(xdg_toplevel::State::Fullscreen));
        surface.send_configure();
    }
    fn parent_changed(&mut self, _: ToplevelSurface) {
        self.scene_dirty = true;
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_unconstrained_geometry(Rectangle::from_size(
                (self.size.width as i32, self.size.height as i32).into(),
            ));
        });
        if let Err(e) = self.popups.track_popup(surface.clone().into()) {
            eprintln!("Popup: {e}");
        }
        if let Err(e) = surface.send_configure() {
            eprintln!("Popup configure: {e}");
        }
    }
    fn grab(&mut self, surface: PopupSurface, _: wl_seat::WlSeat, serial: Serial) {
        if let Some(root) = self.root() {
            let seat = self.seat.clone();
            if let Ok(grab) = self
                .popups
                .grab_popup(root.into(), surface.into(), &seat, serial)
            {
                seat.get_keyboard()
                    .unwrap()
                    .set_grab(self, PopupKeyboardGrab::new(&grab), serial);
                seat.get_pointer().unwrap().set_grab(
                    self,
                    PopupPointerGrab::new(&grab),
                    serial,
                    Focus::Keep,
                );
            }
        }
    }
    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_unconstrained_geometry(Rectangle::from_size(
                (self.size.width as i32, self.size.height as i32).into(),
            ))
        });
        surface.send_repositioned(token);
    }
}
impl XdgDecorationHandler for SingleWindow {
    fn new_decoration(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| {
            state.decoration_mode = Some(zxdg_toplevel_decoration_v1::Mode::ServerSide)
        });
        surface.send_configure();
    }
    fn request_mode(&mut self, surface: ToplevelSurface, _: zxdg_toplevel_decoration_v1::Mode) {
        self.new_decoration(surface);
    }
    fn unset_mode(&mut self, surface: ToplevelSurface) {
        self.new_decoration(surface);
    }
}
smithay::delegate_compositor!(SingleWindow);
smithay::delegate_shm!(SingleWindow);
smithay::delegate_xdg_shell!(SingleWindow);
smithay::delegate_xdg_decoration!(SingleWindow);
smithay::delegate_seat!(SingleWindow);
smithay::delegate_data_device!(SingleWindow);
smithay::delegate_output!(SingleWindow);

#[cfg(feature = "gpu")]
impl SingleWindow {
    pub fn advertise_gpu(&mut self, engine: &RenderEngine) -> Result<(), Box<dyn Error>> {
        use smithay::backend::{
            egl::EGLContext,
            renderer::{gles::GlesRenderer, ImportDma},
        };
        if let RenderEngine::Gl(renderer, _) = engine {
            let context =
                EGLContext::new_shared(renderer.egl_context().display(), renderer.egl_context())?;
            // SAFETY: shared context is created and used only on this compositor thread.
            self.import_renderer = Some(unsafe { GlesRenderer::new(context) }?);
            let formats = renderer.dmabuf_formats();
            if formats.iter().next().is_some() {
                self.dmabuf_state
                    .create_global::<Self>(&self.display, formats.iter().copied());
            }
        }
        Ok(())
    }
}
#[cfg(feature = "gpu")]
impl smithay::wayland::dmabuf::DmabufHandler for SingleWindow {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf_state
    }
    fn dmabuf_imported(
        &mut self,
        _: &smithay::wayland::dmabuf::DmabufGlobal,
        buffer: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: smithay::wayland::dmabuf::ImportNotifier,
    ) {
        use smithay::backend::renderer::ImportDma;
        if self
            .import_renderer
            .as_mut()
            .is_some_and(|renderer| renderer.import_dmabuf(&buffer, None).is_ok())
        {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}
#[cfg(feature = "gpu")]
smithay::delegate_dmabuf!(SingleWindow);
