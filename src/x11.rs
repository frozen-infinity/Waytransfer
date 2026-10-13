//! Minimal rootless XWayland window manager, integrated into the existing compositor thread.
use crate::{compositor::SingleWindow, Dimensions};
use smithay::{
    desktop::Window,
    reexports::{
        calloop::EventLoop,
        wayland_server::{protocol::wl_surface::WlSurface, Client},
    },
    utils::{Logical, Rectangle},
    wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    xwayland::{
        xwm::{Reorder, ResizeEdge, XwmId},
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
    },
};
use std::{error::Error, process::Stdio};
pub fn start(
    display: &smithay::reexports::wayland_server::DisplayHandle,
    event_loop: &EventLoop<'static, SingleWindow>,
) -> Result<(), Box<dyn Error>> {
    let handle = event_loop.handle();
    let wm_handle = handle.clone();
    let (xwayland, client): (_, Client) = XWayland::spawn(
        display,
        None,
        std::iter::empty::<(String, String)>(),
        false,
        Stdio::null(),
        Stdio::inherit(),
        |_| {},
    )?;
    handle.insert_source(xwayland, move |event, _, state| match event {
        XWaylandEvent::Ready {
            x11_socket,
            display_number,
        } => match X11Wm::start_wm(wm_handle.clone(), x11_socket, client.clone()) {
            Ok(wm) => {
                state.xwm = Some(wm);
                state.x11_display = Some(display_number);
                println!("XWayland ready: DISPLAY=:{display_number}");
            }
            Err(e) => eprintln!("XWayland window manager: {e}"),
        },
        XWaylandEvent::Error => eprintln!("XWayland exited during startup"),
    })?;
    Ok(())
}
impl SingleWindow {
    fn x11_id(&self, surface: &X11Surface) -> Option<u32> {
        self.window_entries()
            .find(|(_, window)| window.x11_surface().is_some_and(|x| x == surface))
            .map(|(id, _)| id)
    }
    fn map_x11(&mut self, surface: X11Surface) {
        let geometry = surface.geometry();
        let size = Dimensions::fit(
            geometry.size.w.max(1) as usize,
            geometry.size.h.max(1) as usize,
        );
        if self.x11_id(&surface).is_some() {
            return;
        }
        if let Some(_id) = self.register_window(Window::new_x11_window(surface.clone()), size) {
            if !surface.is_override_redirect() {
                let _ = surface.set_mapped(true);
                let _ = surface.configure(Some(Rectangle::from_size(
                    (size.width as i32, size.height as i32).into(),
                )));
            }
        } else {
            let _ = surface.close();
        }
    }
}
impl XWaylandShellHandler for SingleWindow {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell
    }
    fn surface_associated(&mut self, _: XwmId, _: WlSurface, _: X11Surface) {
        self.mark_all_dirty();
    }
}
impl XwmHandler for SingleWindow {
    fn xwm_state(&mut self, _: XwmId) -> &mut X11Wm {
        self.xwm
            .as_mut()
            .expect("XWM callback requires ready XWayland")
    }
    fn new_window(&mut self, _: XwmId, _: X11Surface) {}
    fn new_override_redirect_window(&mut self, _: XwmId, _: X11Surface) {}
    fn map_window_request(&mut self, _: XwmId, surface: X11Surface) {
        self.map_x11(surface);
    }
    fn mapped_override_redirect_window(&mut self, _: XwmId, surface: X11Surface) {
        self.map_x11(surface);
    }
    fn unmapped_window(&mut self, _: XwmId, surface: X11Surface) {
        if let Some(id) = self.x11_id(&surface) {
            self.remove_window(id);
        }
        if !surface.is_override_redirect() {
            let _ = surface.set_mapped(false);
        }
    }
    fn destroyed_window(&mut self, _: XwmId, surface: X11Surface) {
        if let Some(id) = self.x11_id(&surface) {
            self.remove_window(id);
        }
    }
    fn configure_request(
        &mut self,
        _: XwmId,
        surface: X11Surface,
        _: Option<i32>,
        _: Option<i32>,
        width: Option<u32>,
        height: Option<u32>,
        _: Option<Reorder>,
    ) {
        let old = surface.geometry();
        let size = Dimensions::fit(
            width.unwrap_or(old.size.w.max(1) as u32) as usize,
            height.unwrap_or(old.size.h.max(1) as u32) as usize,
        );
        if let Some(id) = self.x11_id(&surface) {
            let _ = self.input_window(id, crate::Message::Resize { size });
        } else {
            let _ = surface.configure(Some(Rectangle::from_size(
                (size.width as i32, size.height as i32).into(),
            )));
        }
    }
    fn configure_notify(
        &mut self,
        _: XwmId,
        _: X11Surface,
        _: Rectangle<i32, Logical>,
        _: Option<u32>,
    ) {
        self.mark_all_dirty();
    }
    fn property_notify(
        &mut self,
        _: XwmId,
        _: X11Surface,
        _: smithay::xwayland::xwm::WmWindowProperty,
    ) {
        self.mark_metadata_dirty();
    }
    fn resize_request(&mut self, _: XwmId, _: X11Surface, _: u32, _: ResizeEdge) {}
    fn move_request(&mut self, _: XwmId, _: X11Surface, _: u32) {}
}
smithay::delegate_xwayland_shell!(SingleWindow);
