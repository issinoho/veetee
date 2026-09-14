//! CRT effects applied after the page is drawn: phosphor afterglow, a glow
//! around lit dots, and the curvature and vignetting of a tube face.
//!
//! The page is drawn into an offscreen texture. Each frame the afterglow
//! keeps the brighter of the new picture and the previous one faded by the
//! elapsed time, a blurred copy at half size supplies the glow, and a last
//! pass bends and composites the result into the window's framebuffer.
#![allow(unsafe_code)]

use std::time::Instant;

use glow::HasContext;

use crate::theme::Theme;

const FULLSCREEN: &str = r#"
out vec2 v_uv;
void main() {
    // One triangle covering the viewport.
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const PERSIST: &str = r#"
in vec2 v_uv;
uniform sampler2D u_scene;
uniform sampler2D u_previous;
uniform float u_decay;
out vec4 o_color;
void main() {
    vec3 now = texture(u_scene, v_uv).rgb;
    vec3 before = texture(u_previous, v_uv).rgb * u_decay;
    o_color = vec4(max(now, before), 1.0);
}
"#;

const BLUR: &str = r#"
in vec2 v_uv;
uniform sampler2D u_image;
uniform vec2 u_step;
out vec4 o_color;
void main() {
    // A 9-tap Gaussian, sampled between texels to use linear filtering.
    vec3 sum = texture(u_image, v_uv).rgb * 0.2270270;
    sum += texture(u_image, v_uv + u_step * 1.3846154).rgb * 0.3162162;
    sum += texture(u_image, v_uv - u_step * 1.3846154).rgb * 0.3162162;
    sum += texture(u_image, v_uv + u_step * 3.2307692).rgb * 0.0702703;
    sum += texture(u_image, v_uv - u_step * 3.2307692).rgb * 0.0702703;
    o_color = vec4(sum, 1.0);
}
"#;

const COMPOSITE: &str = r#"
in vec2 v_uv;
uniform sampler2D u_image;
uniform sampler2D u_glow;
uniform float u_glow_strength;
uniform float u_curvature;
uniform vec3 u_bezel;
uniform vec2 u_size;
out vec4 o_color;
void main() {
    vec2 uv = v_uv;
    float shade = 1.0;
    if (u_curvature > 0.0) {
        // Barrel distortion about the centre, keeping the window's aspect.
        vec2 c = uv * 2.0 - 1.0;
        vec2 aspect = vec2(u_size.x / u_size.y, 1.0);
        vec2 a = c * aspect;
        c *= 1.0 + u_curvature * dot(a, a) * 0.25;
        uv = c * 0.5 + 0.5;
        if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
            o_color = vec4(u_bezel * 0.6, 1.0);
            return;
        }
        shade = 1.0 - 0.35 * smoothstep(0.6, 1.5, length(a * 0.8));
    }
    vec3 color = texture(u_image, uv).rgb;
    color += texture(u_glow, uv).rgb * u_glow_strength;
    o_color = vec4(color * shade, 1.0);
}
"#;

struct Target {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
}

struct Pass {
    program: glow::Program,
}

impl Pass {
    unsafe fn uniform(&self, gl: &glow::Context, name: &str) -> Option<glow::UniformLocation> {
        unsafe { gl.get_uniform_location(self.program, name) }
    }
}

pub(crate) struct PostFx {
    size: (i32, i32),
    scene: Target,
    persist: [Target; 2],
    front: usize,
    glow: [Target; 2],
    vao: glow::VertexArray,
    persist_pass: Pass,
    blur_pass: Pass,
    composite_pass: Pass,
    last_frame: Option<Instant>,
}

unsafe fn target(gl: &glow::Context, w: i32, h: i32) -> Result<Target, String> {
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w,
            h,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        for (param, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
        }
        let fbo = gl.create_framebuffer()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            return Err("CRT effect framebuffer is incomplete".into());
        }
        Ok(Target { fbo, texture })
    }
}

unsafe fn delete(gl: &glow::Context, t: &Target) {
    unsafe {
        gl.delete_framebuffer(t.fbo);
        gl.delete_texture(t.texture);
    }
}

impl PostFx {
    /// # Safety
    /// A GL context must be current.
    pub(crate) unsafe fn new(
        gl: &glow::Context,
        header: &str,
        w: i32,
        h: i32,
    ) -> Result<PostFx, String> {
        unsafe {
            let program = |fs: &str| -> Result<Pass, String> {
                Ok(Pass {
                    program: crate::gl::link(
                        gl,
                        &format!("{header}{FULLSCREEN}"),
                        &format!("{header}{fs}"),
                    )?,
                })
            };
            let (gw, gh) = ((w / 2).max(1), (h / 2).max(1));
            let fx = PostFx {
                size: (w, h),
                scene: target(gl, w, h)?,
                persist: [target(gl, w, h)?, target(gl, w, h)?],
                front: 0,
                glow: [target(gl, gw, gh)?, target(gl, gw, gh)?],
                vao: gl.create_vertex_array()?,
                persist_pass: program(PERSIST)?,
                blur_pass: program(BLUR)?,
                composite_pass: program(COMPOSITE)?,
                last_frame: None,
            };
            Ok(fx)
        }
    }

    pub(crate) fn size(&self) -> (i32, i32) {
        self.size
    }

    /// Binds the offscreen target the page is drawn into.
    ///
    /// # Safety
    /// The context passed to [`PostFx::new`] must be current.
    pub(crate) unsafe fn begin(&self, gl: &glow::Context) {
        unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.scene.fbo)) };
    }

    /// Applies the effects and draws the result into `output` (the
    /// framebuffer that was bound before [`PostFx::begin`]).
    ///
    /// # Safety
    /// The context passed to [`PostFx::new`] must be current.
    pub(crate) unsafe fn finish(
        &mut self,
        gl: &glow::Context,
        theme: &Theme,
        output: Option<glow::Framebuffer>,
    ) {
        let now = Instant::now();
        let elapsed_ms = self
            .last_frame
            .map_or(1000.0, |t| now.duration_since(t).as_secs_f32() * 1000.0);
        self.last_frame = Some(now);
        let decay = if theme.afterglow_ms > 0.0 {
            (-elapsed_ms / theme.afterglow_ms).exp()
        } else {
            0.0
        };
        let (w, h) = self.size;
        let (gw, gh) = ((w / 2).max(1), (h / 2).max(1));
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);

            // Afterglow: the brighter of now and the faded previous picture.
            let back = 1 - self.front;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.persist[back].fbo));
            gl.viewport(0, 0, w, h);
            let p = &self.persist_pass;
            gl.use_program(Some(p.program));
            gl.uniform_1_i32(p.uniform(gl, "u_scene").as_ref(), 0);
            gl.uniform_1_i32(p.uniform(gl, "u_previous").as_ref(), 1);
            gl.uniform_1_f32(p.uniform(gl, "u_decay").as_ref(), decay);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.scene.texture));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.persist[self.front].texture));
            gl.active_texture(glow::TEXTURE0);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            self.front = back;
            let image = self.persist[self.front].texture;

            // Glow: blur a half-size copy, across then down.
            if theme.glow > 0.0 {
                let b = &self.blur_pass;
                gl.use_program(Some(b.program));
                gl.uniform_1_i32(b.uniform(gl, "u_image").as_ref(), 0);
                gl.viewport(0, 0, gw, gh);
                for (i, (source, step)) in [
                    (image, [1.0 / gw as f32, 0.0]),
                    (self.glow[0].texture, [0.0, 1.0 / gh as f32]),
                ]
                .into_iter()
                .enumerate()
                {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.glow[i].fbo));
                    gl.uniform_2_f32(b.uniform(gl, "u_step").as_ref(), step[0], step[1]);
                    gl.bind_texture(glow::TEXTURE_2D, Some(source));
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            }

            // Composite into the window.
            gl.bind_framebuffer(glow::FRAMEBUFFER, output);
            gl.viewport(0, 0, w, h);
            let c = &self.composite_pass;
            gl.use_program(Some(c.program));
            gl.uniform_1_i32(c.uniform(gl, "u_image").as_ref(), 0);
            gl.uniform_1_i32(c.uniform(gl, "u_glow").as_ref(), 1);
            gl.uniform_1_f32(c.uniform(gl, "u_glow_strength").as_ref(), theme.glow);
            gl.uniform_1_f32(c.uniform(gl, "u_curvature").as_ref(), theme.curvature);
            let [r, g, bl] = theme.bezel;
            gl.uniform_3_f32(c.uniform(gl, "u_bezel").as_ref(), r, g, bl);
            gl.uniform_2_f32(c.uniform(gl, "u_size").as_ref(), w as f32, h as f32);
            gl.bind_texture(glow::TEXTURE_2D, Some(image));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.glow[1].texture));
            gl.active_texture(glow::TEXTURE0);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
        }
    }

    /// # Safety
    /// The context passed to [`PostFx::new`] must be current.
    pub(crate) unsafe fn destroy(self, gl: &glow::Context) {
        unsafe {
            delete(gl, &self.scene);
            for t in self.persist.iter().chain(self.glow.iter()) {
                delete(gl, t);
            }
            gl.delete_vertex_array(self.vao);
            for p in [self.persist_pass, self.blur_pass, self.composite_pass] {
                gl.delete_program(p.program);
            }
        }
    }
}
