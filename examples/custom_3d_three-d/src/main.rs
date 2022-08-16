#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use eframe::egui;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let options = eframe::NativeOptions {
        initial_window_size: Some(egui::vec2(550.0, 610.0)),
        multisampling: 8,
        renderer: eframe::Renderer::Glow,
        depth_buffer: 32,
        ..Default::default()
    };
    eframe::run_native(
        "Custom 3D painting in eframe!",
        options,
        Box::new(|cc| Box::new(MyApp::new(cc))),
    );
}

pub struct MyApp {
    angle: f32,
}

impl MyApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self { angle: 0.2 }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::widgets::global_dark_light_mode_buttons(ui);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label("The triangle is being painted using ");
                ui.hyperlink_to("three-d", "https://github.com/asny/three-d");
                ui.label(".");
            });

            egui::ScrollArea::both().show(ui, |ui| {
                egui::Frame::canvas(ui.style()).show(ui, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::Vec2::splat(512.0), egui::Sense::drag());

                    self.angle += response.drag_delta().x * 0.01;

                    // Clone locals so we can move them into the paint callback:
                    let angle = self.angle;

                    let callback = egui::PaintCallback {
                        rect,
                        callback: std::sync::Arc::new(egui_glow::CallbackFn::new(
                            move |info, painter| {
                                with_three_d_context(painter.gl(), |three_d| {
                                    three_d.custom_painting(info, painter, angle);
                                });
                            },
                        )),
                    };
                    ui.painter().add(callback);
                });
                ui.label("Drag to rotate!");
            });
        });
    }
}

use three_d::*;
struct ThreeDApp {
    context: Context,
}

impl ThreeDApp {
    pub fn new(gl: std::sync::Arc<glow::Context>) -> Self {
        Self {
            context: Context::from_gl_context(gl).unwrap(),
        }
    }
}

impl ThreeDApp {
    fn custom_painting(
        &self,
        info: egui::PaintCallbackInfo,
        painter: &egui_glow::Painter,
        angle: f32,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        #[allow(unsafe_code)]
        unsafe {
            use glow::HasContext as _;
            self.context.disable(glow::FRAMEBUFFER_SRGB);
        }

        let screen = painter
            .intermediate_fbo()
            .map(|fbo| {
                RenderTarget::from_framebuffer(
                    &self.context,
                    info.viewport.width() as u32,
                    info.viewport.height() as u32,
                    fbo,
                )
            })
            .unwrap_or(RenderTarget::screen(
                &self.context,
                info.viewport.width() as u32,
                info.viewport.height() as u32,
            ));

        // Set where to paint
        let viewport = info.viewport_in_pixels();
        let viewport = Viewport {
            x: viewport.left_px.round() as _,
            y: viewport.from_bottom_px.round() as _,
            width: viewport.width_px.round() as _,
            height: viewport.height_px.round() as _,
        };

        // Respect the egui clip region (e.g. if we are inside an `egui::ScrollArea`).
        let clip_rect = info.clip_rect_in_pixels();
        let scissor_box = ScissorBox {
            x: clip_rect.left_px.round() as _,
            y: clip_rect.from_bottom_px.round() as _,
            width: clip_rect.width_px.round() as _,
            height: clip_rect.height_px.round() as _,
        };

        self.render(&screen, viewport, scissor_box, angle);
        screen.into_framebuffer(); // Take back the screen fbo, we will continue to use it.
    }

    fn render(
        &self,
        screen: &three_d::RenderTarget,
        viewport: three_d::Viewport,
        scissor_box: three_d::ScissorBox,
        angle: f32,
    ) {
        // Based on https://github.com/asny/three-d/blob/master/examples/triangle/src/main.rs

        // Create a camera
        let camera = Camera::new_perspective(
            viewport,
            vec3(0.0, 0.0, 2.0),
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
            degrees(45.0),
            0.1,
            10.0,
        );

        // Create a CPU-side mesh consisting of a single colored triangle
        let positions = vec![
            vec3(0.5, -0.5, 0.0),  // bottom right
            vec3(-0.5, -0.5, 0.0), // bottom left
            vec3(0.0, 0.5, 0.0),   // top
        ];
        let colors = vec![
            Color::new(255, 0, 0, 255), // bottom right
            Color::new(0, 255, 0, 255), // bottom left
            Color::new(0, 0, 255, 255), // top
        ];
        let cpu_mesh = CpuMesh {
            positions: Positions::F32(positions),
            colors: Some(colors),
            ..Default::default()
        };

        // Construct a model, with a default color material, thereby transferring the mesh data to the GPU
        let mut model = Gm::new(
            Mesh::new(&self.context, &cpu_mesh),
            ColorMaterial::default(),
        );

        // Set the current transformation of the triangle
        model.set_transformation(Mat4::from_angle_y(radians(angle)));

        // Get the screen render target to be able to render something on the screen
        screen
            // Clear the color and depth of the screen render target
            .clear_partially(scissor_box, ClearState::depth(1.0))
            // Render the triangle with the color material which uses the per vertex colors defined at construction
            .render_partially(scissor_box, &camera, &[&model], &[]);
    }
}

/// We get a [`glow::Context`] from `eframe`, but we want a [`three_d::Context`].
///
/// Sadly we can't just create a [`three_d::Context`] in [`MyApp::new`] and pass it
/// to the [`egui::PaintCallback`] because [`three_d::Context`] isn't `Send+Sync`, which
/// [`egui::PaintCallback`] is.
fn with_three_d_context<R>(
    gl: &std::sync::Arc<glow::Context>,
    f: impl FnOnce(&ThreeDApp) -> R,
) -> R {
    use std::cell::RefCell;
    thread_local! {
        pub static THREE_D: RefCell<Option<ThreeDApp>> = RefCell::new(None);
    }

    THREE_D.with(|three_d| {
        let mut three_d = three_d.borrow_mut();
        let three_d = three_d.get_or_insert_with(|| ThreeDApp::new(gl.clone()));
        f(three_d)
    })
}
