//! Software and optional EGL/GLES offscreen presentation. All access stays on the compositor thread.
use crate::Dimensions;
#[cfg(feature = "gpu")]
use smithay::backend::{
    egl::{native::EGLSurfacelessDisplay, EGLContext, EGLDevice, EGLDisplay},
    renderer::{
        gles::{GlesRenderer, GlesTexture},
        ImportEgl, Texture,
    },
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{surface::WaylandSurfaceRenderElement, AsRenderElements},
            pixman::PixmanRenderer,
            utils::draw_render_elements,
            Bind, Color32F, ExportMem, Frame, ImportAll, Offscreen, Renderer,
        },
    },
    desktop::Window,
    reexports::pixman,
    utils::{Rectangle, Transform},
};
use std::error::Error;
pub enum RenderEngine {
    Software(PixmanRenderer, pixman::Image<'static, 'static>),
    #[cfg(feature = "gpu")]
    Gl(Box<GlesRenderer>, GlesTexture),
}
impl RenderEngine {
    pub fn software() -> Result<Self, Box<dyn Error>> {
        let mut renderer = PixmanRenderer::new()?;
        let image = renderer.create_buffer(Fourcc::Argb8888, (640, 400).into())?;
        Ok(Self::Software(renderer, image))
    }
    #[cfg(feature = "gpu")]
    pub fn gpu(
        display: &smithay::reexports::wayland_server::DisplayHandle,
    ) -> Result<Self, Box<dyn Error>> {
        let mut devices: Vec<_> = EGLDevice::enumerate()?.collect();
        devices.sort_by_key(|device| device.is_software());
        let mut chosen = None;
        for device in devices {
            // SAFETY: owned EGLDevice keeps its native handle alive; this renderer/context
            // are created and used only on this thread, never shared with another renderer.
            if let Ok(egl) = unsafe { EGLDisplay::new(device) } {
                if let Ok(context) = EGLContext::new(&egl) {
                    if let Ok(renderer) = unsafe { GlesRenderer::new(context) } {
                        chosen = Some(renderer);
                        break;
                    }
                }
            }
        }
        let mut renderer = match chosen {
            Some(renderer) => renderer,
            None => {
                // SAFETY: EGLSurfacelessDisplay has no borrowed native resources. Context stays
                // on this compositor thread for its lifetime.
                let egl = unsafe { EGLDisplay::new(EGLSurfacelessDisplay) }?;
                unsafe { GlesRenderer::new(EGLContext::new(&egl)?) }?
            }
        };
        // wl_drm/EGL import is optional on software Mesa. SHM GLES clients can still run.
        if let Err(e) = renderer.bind_wl_display(display) {
            eprintln!("EGL Wayland buffer binding unavailable: {e}");
        }
        let image = renderer.create_buffer(Fourcc::Argb8888, (640, 400).into())?;
        Ok(Self::Gl(Box::new(renderer), image))
    }
    pub fn render(
        &mut self,
        window: Option<&Window>,
        size: Dimensions,
    ) -> Result<Vec<u32>, Box<dyn Error>> {
        match self {
            Self::Software(renderer, image) => {
                if image.width() != size.width as usize || image.height() != size.height as usize {
                    *image = renderer.create_buffer(
                        Fourcc::Argb8888,
                        (size.width as i32, size.height as i32).into(),
                    )?;
                }
                draw(renderer, image, window, size)
            }
            #[cfg(feature = "gpu")]
            Self::Gl(renderer, image) => {
                if image.size().w != size.width as i32 || image.size().h != size.height as i32 {
                    *image = renderer.create_buffer(
                        Fourcc::Argb8888,
                        (size.width as i32, size.height as i32).into(),
                    )?;
                }
                draw(renderer.as_mut(), image, window, size)
            }
        }
    }
}
fn draw<R, B>(
    renderer: &mut R,
    image: &mut B,
    window: Option<&Window>,
    size: Dimensions,
) -> Result<Vec<u32>, Box<dyn Error>>
where
    R: Renderer + ImportAll + Bind<B> + ExportMem,
    R::TextureId: Clone + 'static,
    R::Error: Error + 'static,
{
    let elements: Vec<WaylandSurfaceRenderElement<R>> = window
        .map(|window| {
            window.render_elements(
                renderer,
                (-window.geometry().loc.x, -window.geometry().loc.y).into(),
                1.0.into(),
                1.0,
            )
        })
        .unwrap_or_default();
    let mut target = renderer.bind(image)?;
    let extent = (size.width as i32, size.height as i32).into();
    let damage = Rectangle::from_size(extent);
    let mut frame = renderer.render(&mut target, extent, Transform::Normal)?;
    frame.clear(Color32F::new(0.094, 0.125, 0.173, 1.0), &[damage])?;
    draw_render_elements(&mut frame, 1.0, &elements, &[damage])?;
    frame.finish()?.wait()?;
    let mapping = renderer.copy_framebuffer(
        &target,
        Rectangle::from_size((size.width as i32, size.height as i32).into()),
        Fourcc::Argb8888,
    )?;
    let bytes = renderer.map_texture(&mapping)?;
    // Smithay renders a Normal offscreen target with logical row zero at GL
    // readback row zero. Mapping::flipped describes texture sampling, not a
    // second row reversal for this rendered framebuffer.
    let pixels: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| u32::from_ne_bytes(*pixel) & 0xffffff)
        .collect();
    Ok(pixels)
}
