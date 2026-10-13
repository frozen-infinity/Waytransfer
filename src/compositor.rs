//! Headless, one-toplevel Wayland compositor. Only wl_shm software clients are advertised.
use crate::{Message, HEIGHT, WIDTH};
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
            CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes,
        },
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
        },
        shm::{ShmHandler, ShmState},
    },
};
use std::{collections::BTreeSet, error::Error, os::unix::io::OwnedFd, sync::Arc, time::Instant};

#[derive(Default)]
pub struct ClientState {
    compositor_state: CompositorClientState,
}
impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

pub struct SingleWindow {
    display: DisplayHandle,
    compositor: CompositorState,
    shell: XdgShellState,
    shm: ShmState,
    seats: SeatState<Self>,
    data_device: DataDeviceState,
    _output_manager: OutputManagerState,
    _decorations: XdgDecorationState,
    pub seat: Seat<Self>,
    pub output: Output,
    window: Option<Window>,
    popups: PopupManager,
    dirty: bool,
    start: Instant,
    held: BTreeSet<u16>,
    buttons: [bool; 3],
    active: bool,
}
impl SingleWindow {
    pub fn new(display: &DisplayHandle) -> Result<Self, Box<dyn Error>> {
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
            size: (WIDTH as i32, HEIGHT as i32).into(),
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
            display: display.clone(),
            compositor: CompositorState::new::<Self>(display),
            shell: XdgShellState::new::<Self>(display),
            shm: ShmState::new::<Self>(display, vec![]),
            data_device: DataDeviceState::new::<Self>(display),
            _output_manager: OutputManagerState::new_with_xdg_output::<Self>(display),
            _decorations: XdgDecorationState::new::<Self>(display),
            seats,
            seat,
            output,
            window: None,
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
    fn root(&self) -> Option<WlSurface> {
        self.window
            .as_ref()?
            .toplevel()
            .map(|s| s.wl_surface().clone())
    }
    fn keyboard_focus(&mut self) {
        if self.seat.get_keyboard().unwrap().is_grabbed() {
            return;
        }
        let surface = if self.active { self.root() } else { None };
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
            Message::Focus { active } => {
                if !active {
                    self.release_input();
                }
                self.active = active;
                self.keyboard_focus();
            }
            Message::Pointer { x, y, pressed } => {
                if x as usize >= WIDTH || y as usize >= HEIGHT {
                    return Err("pointer outside surface");
                }
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
        let damage = Rectangle::from_size((WIDTH as i32, HEIGHT as i32).into());
        let mut frame = renderer.render(
            &mut target,
            (WIDTH as i32, HEIGHT as i32).into(),
            Transform::Normal,
        )?;
        frame.clear(Color32F::new(0.094, 0.125, 0.173, 1.0), &[damage])?;
        draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
        frame.finish()?.wait()?;
        let mapping = renderer.copy_framebuffer(
            &target,
            Rectangle::from_size((WIDTH as i32, HEIGHT as i32).into()),
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
            (WIDTH as i32, HEIGHT as i32).into(),
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
        &client.get_data::<ClientState>().unwrap().compositor_state
    }
    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);
        if let Some(window) = &self.window {
            window.on_commit();
            window.output_enter(
                &self.output,
                Rectangle::from_size((WIDTH as i32, HEIGHT as i32).into()),
            );
        }
        self.dirty = true;
    }
    fn destroyed(&mut self, _: &WlSurface) {
        self.dirty = true;
    }
}
impl ShmHandler for SingleWindow {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}
impl OutputHandler for SingleWindow {}
impl SeatHandler for SingleWindow {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seats
    }
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        set_data_device_focus(&self.display, seat, focused.and_then(|s| s.client()));
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
        if self.window.is_some() {
            eprintln!("Only one toplevel is supported; closing additional window");
            surface.send_close();
            return;
        }
        surface.with_pending_state(|state| {
            state.size = Some((WIDTH as i32, HEIGHT as i32).into());
            state.states.set(xdg_toplevel::State::Activated);
            state.states.set(xdg_toplevel::State::Maximized);
        });
        surface.send_configure();
        self.window = Some(Window::new_wayland_window(surface));
        self.dirty = true;
    }
    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if self.root().as_ref() == Some(surface.wl_surface()) {
            self.release_input();
            self.window = None;
            self.dirty = true;
        }
    }
    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_unconstrained_geometry(Rectangle::from_size(
                (WIDTH as i32, HEIGHT as i32).into(),
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
            if let Ok(grab) = self.popups.grab_popup(root, surface.into(), &seat, serial) {
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
                (WIDTH as i32, HEIGHT as i32).into(),
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
