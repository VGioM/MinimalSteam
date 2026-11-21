//! Verre dépoli : copie ce qui est déjà dessiné sous une zone, le floute avec la carte
//! graphique et le redessine à la même place. Le texte et les boutons, dessinés
//! ensuite, restent nets. En cas de souci OpenGL, la zone reste simplement transparente.

use eframe::egui;
use eframe::glow::{self, HasContext};
use std::num::NonZeroU32;

const VERTEX: &str = r#"
out vec2 uv;
uniform vec2 span;
void main() {
    float x = float((gl_VertexID & 1) << 2) - 1.0;
    float y = float((gl_VertexID & 2) << 1) - 1.0;
    uv = vec2((x + 1.0) * 0.5, span.x + span.y * (y + 1.0) * 0.5);
    gl_Position = vec4(x, y, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"
in vec2 uv;
out vec4 color;
uniform sampler2D image;
uniform vec2 step_size;
void main() {
    vec3 s = texture(image, uv).rgb * 0.227027;
    s += (texture(image, uv + step_size * 1.3846).rgb + texture(image, uv - step_size * 1.3846).rgb) * 0.316216;
    s += (texture(image, uv + step_size * 3.2308).rgb + texture(image, uv - step_size * 3.2308).rgb) * 0.070270;
    color = vec4(s, 1.0);
}
"#;

/// Pixels capturés sous la zone, pour que le flou prenne aussi ce qui arrive du bas.
const MARGIN: i32 = 48;

struct Target { texture: glow::Texture, framebuffer: glow::Framebuffer }

struct Gl {
    program: glow::Program,
    vao: glow::VertexArray,
    source: glow::Texture,
    a: Target,
    b: Target,
    span: Option<glow::UniformLocation>,
    step: Option<glow::UniformLocation>,
    image: Option<glow::UniformLocation>,
    size: (i32, i32, i32),
}

#[derive(Default)]
pub struct Backdrop { gl: Option<Gl>, failed: bool }

unsafe fn shader(gl: &glow::Context, kind: u32, head: &str, body: &str) -> Option<glow::Shader> {
    let s = gl.create_shader(kind).ok()?;
    gl.shader_source(s, &format!("{head}{body}"));
    gl.compile_shader(s);
    if gl.get_shader_compile_status(s) { Some(s) } else { gl.delete_shader(s); None }
}

unsafe fn new_texture(gl: &glow::Context) -> Option<glow::Texture> {
    let t = gl.create_texture().ok()?;
    gl.bind_texture(glow::TEXTURE_2D, Some(t));
    for (k, v) in [(glow::TEXTURE_MIN_FILTER, glow::LINEAR), (glow::TEXTURE_MAG_FILTER, glow::LINEAR), (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE), (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE)] {
        gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
    }
    Some(t)
}

unsafe fn allocate(gl: &glow::Context, t: glow::Texture, w: i32, h: i32) {
    gl.bind_texture(glow::TEXTURE_2D, Some(t));
    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
}

unsafe fn new_target(gl: &glow::Context) -> Option<Target> {
    Some(Target { texture: new_texture(gl)?, framebuffer: gl.create_framebuffer().ok()? })
}

unsafe fn build(gl: &glow::Context) -> Option<Gl> {
    let head = if gl.version().is_embedded { "#version 300 es\nprecision mediump float;\n" } else { "#version 330\n" };
    let vs = shader(gl, glow::VERTEX_SHADER, head, VERTEX)?;
    let fs = shader(gl, glow::FRAGMENT_SHADER, head, FRAGMENT)?;
    let program = gl.create_program().ok()?;
    gl.attach_shader(program, vs);
    gl.attach_shader(program, fs);
    gl.link_program(program);
    gl.delete_shader(vs);
    gl.delete_shader(fs);
    if !gl.get_program_link_status(program) { gl.delete_program(program); return None; }
    Some(Gl {
        span: gl.get_uniform_location(program, "span"),
        step: gl.get_uniform_location(program, "step_size"),
        image: gl.get_uniform_location(program, "image"),
        program,
        vao: gl.create_vertex_array().ok()?,
        source: new_texture(gl)?,
        a: new_target(gl)?,
        b: new_target(gl)?,
        size: (0, 0, 0),
    })
}

/// (réduction de l'image, nombre de passes) selon le niveau de flou (1 à 100).
pub fn strength(level: u8) -> (i32, u32) {
    let level = level.clamp(1, 100) as i32;
    (2 + level / 34, 1 + level as u32 / 20)
}

impl Backdrop {
    pub fn paint(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo, level: u8) {
        if self.failed || level == 0 { return; }
        unsafe {
            if self.gl.is_none() {
                self.gl = build(gl);
                if self.gl.is_none() { self.failed = true; return; }
            }
            let Some(s) = self.gl.as_mut() else { return };
            // À lire avant tout changement de cible : c'est là qu'egui continuera de dessiner.
            let screen = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
            let screen = NonZeroU32::new(screen as u32).map(glow::NativeFramebuffer);
            let view = info.viewport_in_pixels();
            let clip = info.clip_rect_in_pixels();
            if view.width_px <= 0 || view.height_px <= 0 { return; }
            let below = MARGIN.min(view.from_bottom_px).max(0);
            let (w, h) = (view.width_px, view.height_px + below);
            let (scale, passes) = strength(level);
            let (small_w, small_h) = ((w / scale).max(1), (h / scale).max(1));
            if s.size != (w, h, scale) {
                allocate(gl, s.source, w, h);
                allocate(gl, s.a.texture, small_w, small_h);
                allocate(gl, s.b.texture, small_w, small_h);
                for t in [&s.a, &s.b] {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.framebuffer));
                    gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(t.texture), 0);
                }
                s.size = (w, h, scale);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(s.source));
            gl.copy_tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, view.left_px, view.from_bottom_px - below, w, h);

            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.use_program(Some(s.program));
            gl.bind_vertex_array(Some(s.vao));
            gl.uniform_1_i32(s.image.as_ref(), 0);
            gl.uniform_2_f32(s.span.as_ref(), 0.0, 1.0);
            gl.viewport(0, 0, small_w, small_h);

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(s.a.framebuffer));
            gl.uniform_2_f32(s.step.as_ref(), 0.0, 0.0);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            let (dx, dy) = (1.0 / small_w as f32, 1.0 / small_h as f32);
            for _ in 0..passes {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(s.b.framebuffer));
                gl.bind_texture(glow::TEXTURE_2D, Some(s.a.texture));
                gl.uniform_2_f32(s.step.as_ref(), dx, 0.0);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(s.a.framebuffer));
                gl.bind_texture(glow::TEXTURE_2D, Some(s.b.texture));
                gl.uniform_2_f32(s.step.as_ref(), 0.0, dy);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }

            gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
            gl.viewport(view.left_px, view.from_bottom_px, view.width_px, view.height_px);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px);
            gl.bind_texture(glow::TEXTURE_2D, Some(s.a.texture));
            gl.uniform_2_f32(s.step.as_ref(), 0.0, 0.0);
            gl.uniform_2_f32(s.span.as_ref(), below as f32 / h as f32, view.height_px as f32 / h as f32);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stronger_level_blurs_more() {
        assert_eq!(strength(0), strength(1));
        assert!(strength(100).0 > strength(1).0);
        assert!(strength(100).1 > strength(1).1);
    }
}
