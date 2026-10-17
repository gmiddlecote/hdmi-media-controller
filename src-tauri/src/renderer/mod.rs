//! Output rendering using glium/OpenGL instead of webviews.
//!
//! Each output is a native OpenGL window created on a dedicated thread.
//! Images are decoded with the [`image`] crate and uploaded as OpenGL
//! textures. Cover/contain fit is applied by computing the quad's vertex
//! positions in Rust.
//!
//! Video decoding is not yet available offline. Video items render an error
//! message instead of playing.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use glium::glutin::platform::run_return::EventLoopExtRunReturn;
use glium::{glutin, Surface};

use crate::logging;
use crate::media::{MediaItem, ObjectFit};

/// Commands sent from the scheduler to the renderer thread.
#[derive(Debug)]
enum RendererCommand {
    /// Close the output window.
    Close,
    /// Update the fit mode.
    SetFit(ObjectFit),
    /// Update the overlay text (stored, not yet drawn).
    SetOverlay(String),
}

/// One open output window and its rendering state, owned by the renderer
/// thread.
struct Session {
    display: glium::Display,
    /// Current media texture (None while loading or on error).
    texture: Option<glium::texture::Texture2d>,
    texture_size: (u32, u32),
    item: MediaItem,
    fit: ObjectFit,
    /// Overlay text to draw over the media (stored, not yet drawn).
    overlay: String,
    #[allow(dead_code)]
    overlay_v: String,
    #[allow(dead_code)]
    overlay_h: String,
    /// Program + buffers for drawing media.
    program: glium::Program,
    ib: glium::index::IndexBuffer<u16>,
}

/// A vertex: 2D position + texture coordinates.
#[derive(Copy, Clone, Debug)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

glium::implement_vertex!(Vertex, position, tex_coords);

/// Media vertex shader: passthrough positions and texture coords.
const MEDIA_VERTEX_SRC: &str = r#"#version 140
in vec2 position;
in vec2 tex_coords;
out vec2 v_tex_coords;
void main() {
    v_tex_coords = tex_coords;
    gl_Position = vec4(position, 0.0, 1.0);
}"#;

/// Media fragment shader: sample the texture.
const MEDIA_FRAGMENT_SRC: &str = r#"#version 140
in vec2 v_tex_coords;
out vec4 color;
uniform sampler2D tex;
uniform vec4 tint;
void main() {
    vec4 t = texture(tex, v_tex_coords);
    color = vec4(t.rgb * tint.rgb, t.a * tint.a);
}"#;

/// Fullscreen quad covering [-1, 1] in both axes.
const FULLSCREEN_QUAD: [Vertex; 4] = [
    Vertex {
        position: [-1.0, -1.0],
        tex_coords: [0.0, 1.0],
    },
    Vertex {
        position: [1.0, -1.0],
        tex_coords: [1.0, 1.0],
    },
    Vertex {
        position: [1.0, 1.0],
        tex_coords: [1.0, 0.0],
    },
    Vertex {
        position: [-1.0, 1.0],
        tex_coords: [0.0, 0.0],
    },
];
const QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

/// Tracked by the scheduler: one sender per open output label.
#[derive(Default)]
pub struct RendererState(Mutex<HashMap<String, RendererHandle>>);

/// A handle to a renderer thread: send commands, join on shutdown.
pub struct RendererHandle {
    tx: mpsc::Sender<RendererCommand>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RendererState {
    /// Returns the label of every open output window.
    pub fn labels(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().cloned().collect()
    }
}

/// Playback volume shared by every output window, from 0.0 (silent) to 1.0
/// (full).
pub struct Volume(Mutex<f32>);

impl Default for Volume {
    fn default() -> Self {
        Self(Mutex::new(1.0))
    }
}

impl Volume {
    /// The level output windows should play at.
    pub fn get(&self) -> f32 {
        *self.0.lock().unwrap()
    }

    /// Stores a new level, clamped to 0.0–1.0.
    pub fn set(&self, level: f32) {
        *self.0.lock().unwrap() = level.clamp(0.0, 1.0);
    }
}

pub const PREVIEW_LABEL: &str = "preview";

/// Errors produced while opening or managing output windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererError {
    NoDisplayBounds(String),
    Creation(String),
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDisplayBounds(name) => {
                write!(f, "Display {name} has no usable screen bounds.")
            }
            Self::Creation(detail) => write!(f, "Could not open output window: {detail}"),
        }
    }
}

impl std::error::Error for RendererError {}

/// Creates a window label for a display, e.g. `output-DISPLAY1-3`.
pub fn label_for(device_name: &str, seq: u64) -> String {
    let name = device_name
        .rsplit('\\')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("display");
    format!("output-{name}-{seq}")
}

/// Opens a borderless fullscreen OpenGL window on `display` playing `item`.
pub fn open(
    renderer: &RendererState,
    display: &crate::display::model::DisplayInfo,
    item: &MediaItem,
    label: &str,
    overlay: &str,
) -> Result<(), RendererError> {
    let bounds = display.bounds;
    if bounds.width == 0 || bounds.height == 0 {
        return Err(RendererError::NoDisplayBounds(display.device_name.clone()));
    }

    let (tx, rx) = mpsc::channel::<RendererCommand>();
    let label = label.to_string();
    let item = item.clone();
    let overlay = overlay.to_string();
    let x = bounds.x;
    let y = bounds.y;
    let width = bounds.width;
    let height = bounds.height;
    let label_for_insert = label.clone();

    let thread = thread::spawn(move || {
        run_renderer(rx, label, item, overlay, x, y, width, height);
    });

    renderer.0.lock().unwrap().insert(
        label_for_insert,
        RendererHandle {
            tx,
            thread: Some(thread),
        },
    );

    Ok(())
}

pub fn open_preview(_renderer: &RendererState, _item: &MediaItem) -> Result<(), RendererError> {
    Err(RendererError::Creation(
        "video preview is not supported without a video decoder".into(),
    ))
}

pub fn close_preview(_renderer: &RendererState) -> bool {
    false
}

/// Opens an output window that plays `item`. Audio is dropped in this
/// version, so audio items render an error message on a visible window.
pub fn open_audio(
    renderer: &RendererState,
    item: &MediaItem,
    label: &str,
    overlay: &str,
) -> Result<(), RendererError> {
    let displays = crate::display::list_displays().unwrap_or_default();
    let Some(display) = displays.first() else {
        return Err(RendererError::NoDisplayBounds("no display".into()));
    };
    open(renderer, display, item, label, overlay)
}

/// Closes the output with `label`. Returns whether a window was open.
pub fn close(renderer: &RendererState, label: &str) -> bool {
    let handle = {
        let mut sessions = renderer.0.lock().unwrap();
        sessions.remove(label)
    };
    let Some(handle) = handle else {
        return false;
    };

    let _ = handle.tx.send(RendererCommand::Close);
    if let Some(thread) = handle.thread {
        let _ = thread.join();
    }
    true
}

/// Closes every open output window; returns how many were closed.
pub fn close_all(renderer: &RendererState) -> usize {
    let labels = renderer
        .0
        .lock()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut count = 0;
    for label in labels {
        if close(renderer, &label) {
            count += 1;
        }
    }
    count
}

/// Refreshes the stored fit mode for an open window.
pub fn set_fit(renderer: &RendererState, label: &str, fit: ObjectFit) -> bool {
    let tx = {
        let sessions = renderer.0.lock().unwrap();
        sessions.get(label).map(|h| h.tx.clone())
    };
    let Some(tx) = tx else {
        return false;
    };
    let _ = tx.send(RendererCommand::SetFit(fit));
    true
}

/// Updates the overlay text for an open window.
pub fn set_overlay(renderer: &RendererState, label: &str, text: String) -> bool {
    let tx = {
        let sessions = renderer.0.lock().unwrap();
        sessions.get(label).map(|h| h.tx.clone())
    };
    let Some(tx) = tx else {
        return false;
    };
    let _ = tx.send(RendererCommand::SetOverlay(text));
    true
}

/// Number of output windows currently open.
pub fn open_count(renderer: &RendererState) -> usize {
    renderer.0.lock().unwrap().len()
}

/// The renderer thread entry point.
#[allow(clippy::too_many_arguments)]
fn run_renderer(
    rx: mpsc::Receiver<RendererCommand>,
    label: String,
    item: MediaItem,
    overlay: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) {
    let mut event_loop = glutin::event_loop::EventLoop::new();

    let window_builder = glutin::window::WindowBuilder::new()
        .with_title(format!("DuetPlay \u{2014} {}", item.name))
        .with_decorations(false)
        .with_resizable(false)
        .with_always_on_top(true)
        .with_visible(true)
        .with_position(glutin::dpi::PhysicalPosition::new(x, y))
        .with_inner_size(glutin::dpi::PhysicalSize::new(width, height));

    let context_builder = glutin::ContextBuilder::new().with_vsync(true);

    let display = match glium::Display::new(window_builder, context_builder, &event_loop) {
        Ok(d) => d,
        Err(e) => {
            logging::error(&format!(
                "renderer {label}: failed to create glium display: {e}"
            ));
            return;
        }
    };

    let media_program =
        glium::Program::from_source(&display, MEDIA_VERTEX_SRC, MEDIA_FRAGMENT_SRC, None).unwrap();

    let ib = glium::index::IndexBuffer::new(
        &display,
        glium::index::PrimitiveType::TrianglesList,
        &QUAD_INDICES,
    )
    .unwrap();

    let mut session = Session {
        display,
        texture: None,
        texture_size: (0, 0),
        item,
        fit: ObjectFit::Cover,
        overlay,
        overlay_v: "bottom".into(),
        overlay_h: "left".into(),
        program: media_program,
        ib,
    };

    // Load the initial item's texture.
    load_item_texture(&mut session);

    let mut running = true;
    while running {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                RendererCommand::Close => {
                    running = false;
                }
                RendererCommand::SetFit(fit) => {
                    session.fit = fit;
                    session.display.gl_window().window().request_redraw();
                }
                RendererCommand::SetOverlay(text) => {
                    session.overlay = text;
                    session.display.gl_window().window().request_redraw();
                }
            }
        }

        if !running {
            break;
        }

        event_loop.run_return(|event, _, control_flow| {
            *control_flow = glutin::event_loop::ControlFlow::Poll;
            match event {
                glutin::event::Event::WindowEvent {
                    event: glutin::event::WindowEvent::CloseRequested,
                    ..
                } => {
                    *control_flow = glutin::event_loop::ControlFlow::Exit;
                }
                glutin::event::Event::RedrawRequested(_) => {
                    render_frame(&mut session);
                }
                _ => {}
            }
        });

        if !running {
            break;
        }

        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Decodes the session's current item and uploads it as a texture.
fn load_item_texture(session: &mut Session) {
    let path = Path::new(&session.item.path);
    let result = (|| -> Result<glium::texture::Texture2d, String> {
        let img = image::open(path).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();
        let dimensions = rgba.dimensions();
        let raw_image = glium::texture::RawImage2d::from_raw_rgba(rgba.into_raw(), dimensions);
        glium::texture::Texture2d::new(&session.display, raw_image).map_err(|e| e.to_string())
    })();

    match result {
        Ok(texture) => {
            let dims = texture.dimensions();
            session.texture_size = (dims.0, dims.1);
            session.texture = Some(texture);
        }
        Err(e) => {
            logging::error(&format!(
                "renderer {}: failed to load image {}: {}",
                session.item.name, session.item.path, e
            ));
            session.texture = None;
        }
    }
}

/// Renders a single frame for the session.
fn render_frame(session: &mut Session) {
    let display = &session.display;
    let window_size = display.gl_window().window().inner_size();
    let mut target = display.draw();

    target.clear_color_and_depth((0.0, 0.0, 0.0, 1.0), 1.0);

    let vertices = fit_quad(
        session.fit,
        session.texture_size.0 as f32,
        session.texture_size.1 as f32,
        window_size.width as f32,
        window_size.height as f32,
    );

    if let Some(ref texture) = session.texture {
        let vb = glium::vertex::VertexBuffer::new(display, &vertices).unwrap();

        target
            .draw(
                &vb,
                &session.ib,
                &session.program,
                &glium::uniform! {
                    tex: texture,
                    tint: [1.0f32, 1.0, 1.0, 1.0],
                },
                &glium::DrawParameters {
                    blend: glium::draw_parameters::Blend::alpha_blending(),
                    ..Default::default()
                },
            )
            .unwrap();
    } else {
        draw_error_placeholder(session, &mut target);
    }

    target.finish().unwrap();
}

/// Computes the vertex positions for a quad that fills the screen according
/// to the fit mode. Texture coordinates always span [0,1]x[0,1].
fn fit_quad(fit: ObjectFit, tex_w: f32, tex_h: f32, screen_w: f32, screen_h: f32) -> Vec<Vertex> {
    if tex_w == 0.0 || tex_h == 0.0 {
        return FULLSCREEN_QUAD.to_vec();
    }

    let tex_aspect = tex_w / tex_h;
    let screen_aspect = screen_w / screen_h;

    let scale = match fit {
        ObjectFit::Cover => {
            if tex_aspect > screen_aspect {
                screen_h / tex_h
            } else {
                screen_w / tex_w
            }
        }
        ObjectFit::Contain => {
            if tex_aspect > screen_aspect {
                screen_w / tex_w
            } else {
                screen_h / tex_h
            }
        }
    };

    let scaled_w = tex_w * scale;
    let scaled_h = tex_h * scale;
    let left = (screen_w - scaled_w) / 2.0;
    let top = (screen_h - scaled_h) / 2.0;

    let l = left / screen_w * 2.0 - 1.0;
    let r = (left + scaled_w) / screen_w * 2.0 - 1.0;
    let b = (screen_h - top) / screen_h * 2.0 - 1.0;
    let t = (screen_h - top - scaled_h) / screen_h * 2.0 - 1.0;

    vec![
        Vertex {
            position: [l, b],
            tex_coords: [0.0, 1.0],
        },
        Vertex {
            position: [r, b],
            tex_coords: [1.0, 1.0],
        },
        Vertex {
            position: [r, t],
            tex_coords: [1.0, 0.0],
        },
        Vertex {
            position: [l, t],
            tex_coords: [0.0, 0.0],
        },
    ]
}

/// Draws an error placeholder when no texture is available.
fn draw_error_placeholder(session: &Session, target: &mut glium::Frame) {
    let screen_w = session.display.gl_window().window().inner_size().width as f32;
    let screen_h = session.display.gl_window().window().inner_size().height as f32;

    let quad_w = 600.0;
    let quad_h = 120.0;
    let l = (screen_w - quad_w) / 2.0 / screen_w * 2.0 - 1.0;
    let r = l + quad_w / screen_w * 2.0;
    let b = 0.0;
    let t = -quad_h / screen_h * 2.0;

    let vertices = [
        Vertex {
            position: [l, b],
            tex_coords: [0.0, 1.0],
        },
        Vertex {
            position: [r, b],
            tex_coords: [1.0, 1.0],
        },
        Vertex {
            position: [r, t],
            tex_coords: [1.0, 0.0],
        },
        Vertex {
            position: [l, t],
            tex_coords: [0.0, 0.0],
        },
    ];

    let vb = glium::vertex::VertexBuffer::new(&session.display, &vertices).unwrap();
    let ib = glium::index::IndexBuffer::new(
        &session.display,
        glium::index::PrimitiveType::TrianglesList,
        &QUAD_INDICES,
    )
    .unwrap();

    let white_texture = glium::texture::Texture2d::new(
        &session.display,
        glium::texture::RawImage2d::from_raw_rgba(vec![255u8, 255, 255, 255], (1, 1)),
    )
    .unwrap();

    target
        .draw(
            &vb,
            &ib,
            &session.program,
            &glium::uniform! {
                tex: white_texture,
                tint: [0.2f32, 0.2, 0.2, 1.0],
            },
            &Default::default(),
        )
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_unique_labels_from_device_names() {
        assert_eq!(label_for(r"\\.\DISPLAY1", 1), "output-DISPLAY1-1");
        assert_eq!(label_for(r"\\.\DISPLAY2", 3), "output-DISPLAY2-3");
        assert_eq!(label_for("no-backslashes", 0), "output-no-backslashes-0");
    }

    #[test]
    fn label_contains_only_safe_characters() {
        let label = label_for(r"\\.\DISPLAY3", 42);
        for ch in label.chars() {
            assert!(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
        }
    }

    #[test]
    fn fit_quad_cover_fullscreen() {
        let verts = fit_quad(ObjectFit::Cover, 100.0, 100.0, 200.0, 100.0);
        assert!((verts[0].position[0] + 1.0).abs() < 0.01);
        assert!((verts[1].position[0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn fit_quad_contain_fullscreen() {
        let verts = fit_quad(ObjectFit::Contain, 100.0, 100.0, 200.0, 100.0);
        assert!((verts[0].position[0] + 0.5).abs() < 0.01);
        assert!((verts[1].position[0] - 0.5).abs() < 0.01);
    }
}
