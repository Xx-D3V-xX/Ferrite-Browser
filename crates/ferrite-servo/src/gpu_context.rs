// A GPU-backed offscreen rendering context for the engine.
//
// Portions of this file are adapted from `servo-paint-api`'s
// `SoftwareRenderingContext` and its private `SurfmanRenderingContext`
// (servo 0.6.0), which are under the Mozilla Public License 2.0
// (https://mozilla.org/MPL/2.0/). This file therefore stays under the MPL-2.0;
// the rest of the crate is MIT OR Apache-2.0.
//
// What differs from the engine's own `SoftwareRenderingContext`:
//
// * it asks surfman for the *hardware* adapter. The engine's software context
//   forces a CPU rasteriser (on macOS, Apple's generic software renderer),
//   which the engine documents as "generally bad performance"; WebRender then
//   rasterises every page on the CPU at the screen's full pixel count;
// * it proves the GPU path works before the app relies on it: it clears the
//   surface to a known colour and reads it back. Any failure is an `Err`, and
//   the caller falls back to the software context;
// * reading a frame back flips it in place rather than copying it first.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use euclid::default::Size2D as UntypedSize2D;
use euclid::Size2D;
use gleam::gl::{self, Gl};
use servo::{DeviceIntRect, RenderingContext, RgbaImage};
use surfman::chains::{PreserveBuffer, SwapChain};
use surfman::{
    Connection, Context, ContextAttributeFlags, ContextAttributes, Device, Error, GLApi, GLVersion,
    Surface, SurfaceAccess, SurfaceInfo, SurfaceTexture, SurfaceType,
};
use winit::dpi::PhysicalSize;

/// An offscreen GL context on the GPU that the engine renders into and the app
/// reads pixels out of.
pub struct HardwareRenderingContext {
    size: Cell<PhysicalSize<u32>>,
    gleam_gl: Rc<dyn Gl>,
    glow_gl: Arc<glow::Context>,
    device: RefCell<Device>,
    context: RefCell<Context>,
    swap_chain: SwapChain<Device>,
    /// What the driver calls itself, for the log.
    renderer: String,
}

impl Drop for HardwareRenderingContext {
    fn drop(&mut self) {
        let device = &mut self.device.borrow_mut();
        let context = &mut self.context.borrow_mut();
        let _ = device.destroy_context(context);
    }
}

impl HardwareRenderingContext {
    /// Creates the context, or says why the GPU path cannot be used.
    pub fn new(size: PhysicalSize<u32>) -> Result<Self, Error> {
        if size.width == 0 || size.height == 0 {
            return Err(Error::Failed);
        }
        let connection = Connection::new()?;
        let adapter = connection.create_hardware_adapter()?;
        let device = connection.create_device(&adapter)?;

        let flags = ContextAttributeFlags::ALPHA
            | ContextAttributeFlags::DEPTH
            | ContextAttributeFlags::STENCIL;
        let gl_api = connection.gl_api();
        let version = match &gl_api {
            GLApi::GLES => GLVersion { major: 3, minor: 0 },
            GLApi::GL => GLVersion { major: 3, minor: 2 },
        };
        let descriptor = device.create_context_descriptor(&ContextAttributes { flags, version })?;
        let context = device.create_context(&descriptor, None)?;

        #[allow(unsafe_code)]
        let gleam_gl = match gl_api {
            // SAFETY: the loader asks the context just created for its own symbols.
            GLApi::GL => unsafe {
                gl::GlFns::load_with(|name| device.get_proc_address(&context, name))
            },
            // SAFETY: as above.
            GLApi::GLES => unsafe {
                gl::GlesFns::load_with(|name| device.get_proc_address(&context, name))
            },
        };
        #[allow(unsafe_code)]
        // SAFETY: as above.
        let glow_gl = unsafe {
            glow::Context::from_loader_function(|name| device.get_proc_address(&context, name))
        };

        let surface = device.create_surface(
            &context,
            SurfaceAccess::GPUOnly,
            SurfaceType::Generic {
                size: Size2D::new(size.width as i32, size.height as i32),
            },
        )?;
        let mut context = context;
        device
            .bind_surface_to_context(&mut context, surface)
            .map_err(|(err, mut surface)| {
                let _ = device.destroy_surface(&mut context, &mut surface);
                err
            })?;
        device.make_context_current(&context)?;
        let swap_chain = SwapChain::create_attached(&device, &mut context, SurfaceAccess::GPUOnly)?;
        let renderer = gleam_gl.get_string(gl::RENDERER);

        let this = Self {
            size: Cell::new(size),
            gleam_gl,
            glow_gl: Arc::new(glow_gl),
            device: RefCell::new(device),
            context: RefCell::new(context),
            swap_chain,
            renderer,
        };
        this.self_test()?;
        Ok(this)
    }

    /// Clears the surface to a colour no blank default would be and reads it
    /// back. A GPU path that cannot do that is not one to hand pages to.
    fn self_test(&self) -> Result<(), Error> {
        self.prepare_for_rendering();
        self.gleam_gl.clear_color(1.0, 0.0, 0.0, 1.0);
        self.gleam_gl.clear(gl::COLOR_BUFFER_BIT);
        self.gleam_gl.finish();
        let rect = DeviceIntRect::from_origin_and_size(
            euclid::Point2D::origin(),
            euclid::Size2D::new(2, 2),
        );
        let image = self.read_frame(rect).ok_or(Error::Failed)?;
        let [r, g, b, a] = image.get_pixel(0, 0).0;
        if r >= 250 && g <= 5 && b <= 5 && a >= 250 {
            Ok(())
        } else {
            Err(Error::Failed)
        }
    }

    /// What the GPU driver calls itself (`GL_RENDERER`).
    #[must_use]
    pub fn renderer(&self) -> &str {
        &self.renderer
    }

    fn framebuffer_id(&self) -> u32 {
        let device = self.device.borrow();
        let context = self.context.borrow();
        device
            .context_surface_info(&context)
            .unwrap_or(None)
            .and_then(|info| info.framebuffer_object)
            .map_or(0, |fb| fb.0.into())
    }

    /// Reads `rect` of the bound framebuffer, top row first.
    fn read_frame(&self, rect: DeviceIntRect) -> Option<RgbaImage> {
        let gl = &self.gleam_gl;
        gl.bind_framebuffer(gl::FRAMEBUFFER, self.framebuffer_id());
        gl.bind_vertex_array(0);
        let mut pixels = gl.read_pixels(
            rect.min.x,
            rect.min.y,
            rect.width(),
            rect.height(),
            gl::RGBA,
            gl::UNSIGNED_BYTE,
        );
        let error = gl.get_error();
        if error != gl::NO_ERROR {
            log::warn!("GL error 0x{error:x} after read_pixels");
        }
        // GL's rows run bottom to top; swap them in place (no second copy).
        let stride = rect.width() as usize * 4;
        let rows = rect.height() as usize;
        for y in 0..rows / 2 {
            let (top, bottom) = pixels.split_at_mut((rows - 1 - y) * stride);
            top[y * stride..(y + 1) * stride].swap_with_slice(&mut bottom[..stride]);
        }
        RgbaImage::from_raw(rect.width() as u32, rect.height() as u32, pixels)
    }
}

impl RenderingContext for HardwareRenderingContext {
    fn prepare_for_rendering(&self) {
        self.gleam_gl
            .bind_framebuffer(gl::FRAMEBUFFER, self.framebuffer_id());
    }

    fn read_to_image(&self, source_rectangle: DeviceIntRect) -> Option<RgbaImage> {
        self.read_frame(source_rectangle)
    }

    fn size(&self) -> PhysicalSize<u32> {
        self.size.get()
    }

    fn resize(&self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 || self.size.get() == size {
            return;
        }
        self.size.set(size);
        let device = &mut self.device.borrow_mut();
        let context = &mut self.context.borrow_mut();
        let size = Size2D::new(size.width as i32, size.height as i32);
        let _ = self.swap_chain.resize(device, context, size);
    }

    fn present(&self) {
        let device = &mut self.device.borrow_mut();
        let context = &mut self.context.borrow_mut();
        let _ = self
            .swap_chain
            .swap_buffers(device, context, PreserveBuffer::No);
    }

    fn make_current(&self) -> Result<(), Error> {
        self.device
            .borrow()
            .make_context_current(&self.context.borrow())
    }

    fn gleam_gl_api(&self) -> Rc<dyn Gl> {
        self.gleam_gl.clone()
    }

    fn glow_gl_api(&self) -> Arc<glow::Context> {
        self.glow_gl.clone()
    }

    fn create_texture(
        &self,
        surface: Surface,
    ) -> Option<(SurfaceTexture, u32, UntypedSize2D<i32>)> {
        let device = self.device.borrow();
        let context = &mut self.context.borrow_mut();
        let SurfaceInfo { size, .. } = device.surface_info(&surface);
        let surface_texture = device.create_surface_texture(context, surface).ok()?;
        let gl_texture = device
            .surface_texture_object(&surface_texture)
            .map_or(0, |tex| tex.0.get());
        Some((surface_texture, gl_texture, size))
    }

    fn destroy_texture(&self, surface_texture: SurfaceTexture) -> Option<Surface> {
        let device = self.device.borrow();
        let context = &mut self.context.borrow_mut();
        device
            .destroy_surface_texture(context, surface_texture)
            .map_err(|(error, _)| error)
            .ok()
    }

    fn connection(&self) -> Option<Connection> {
        Some(self.device.borrow().connection())
    }
}
