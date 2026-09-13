//! OpenGL (3.3 core or ES 3.0) drawing. glow's API is `unsafe` because GL
//! calls require a current context; the caller guarantees one is current.
#![allow(unsafe_code)]

use glow::HasContext;
use vt_core::Terminal;
use vt_fonts::FontSet;

use crate::scene::{
    FrameState, INSTANCE_LEN, Layout, SOFT_ATLAS_COLUMNS, SOFT_SLOT, SoftAtlas, build_instances,
};
use crate::theme::Theme;

/// Glyph slots per atlas row; every face's glyphs share one atlas of
/// 16×16 slots, the largest cell.
const ATLAS_COLUMNS: i32 = 64;
const ATLAS_SLOT: i32 = 16;

pub struct Renderer {
    program: glow::Program,
    vao: glow::VertexArray,
    quad: glow::Buffer,
    instances: glow::Buffer,
    atlas: glow::Texture,
    soft_texture: glow::Texture,
    soft: SoftAtlas,
    fonts: FontSet,
    scratch: Vec<f32>,
    uniforms: Uniforms,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer").finish_non_exhaustive()
    }
}

struct Uniforms {
    viewport: Option<glow::UniformLocation>,
    atlas: Option<glow::UniformLocation>,
    soft_atlas: Option<glow::UniformLocation>,
    slot: Option<glow::UniformLocation>,
    cell_scan_lines: Option<glow::UniformLocation>,
    soft_slot: Option<glow::UniformLocation>,
    soft_columns: Option<glow::UniformLocation>,
    atlas_columns: Option<glow::UniformLocation>,
    scanlines: Option<glow::UniformLocation>,
    stretch: Option<glow::UniformLocation>,
}

const VERTEX: &str = r#"
layout(location = 0) in vec2 a_corner;
layout(location = 1) in vec4 a_rect;
layout(location = 2) in vec2 a_glyph_flags;
layout(location = 3) in vec3 a_fg;
layout(location = 4) in vec3 a_bg;
layout(location = 5) in vec2 a_matrix;

uniform vec2 u_viewport;

flat out vec2 v_origin;
flat out int v_glyph;
flat out int v_flags;
flat out vec3 v_fg;
flat out vec3 v_bg;
flat out vec2 v_size;
flat out ivec2 v_matrix;

void main() {
    vec2 p = a_rect.xy + a_corner * a_rect.zw;
    v_origin = a_rect.xy;
    v_glyph = int(a_glyph_flags.x + 0.5);
    v_flags = int(a_glyph_flags.y + 0.5);
    v_fg = a_fg;
    v_bg = a_bg;
    v_size = a_rect.zw;
    v_matrix = ivec2(a_matrix + 0.5);
    gl_Position = vec4(p.x / u_viewport.x * 2.0 - 1.0, 1.0 - p.y / u_viewport.y * 2.0, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"
flat in vec2 v_origin;
flat in int v_glyph;
flat in int v_flags;
flat in vec3 v_fg;
flat in vec3 v_bg;
flat in vec2 v_size;
flat in ivec2 v_matrix;

uniform vec2 u_viewport;
uniform sampler2D u_atlas;
uniform sampler2D u_soft_atlas;
uniform int u_slot;
uniform int u_atlas_columns;
uniform float u_cell_scan_lines;
uniform int u_soft_slot;
uniform int u_soft_columns;
uniform float u_scanlines;
uniform bool u_stretch;

out vec4 o_color;

float dot_at(int x, int y) {
    if (x < 0 || y < 0 || x >= v_matrix.x || y >= v_matrix.y) return 0.0;
    if ((v_flags & 256) != 0) {
        ivec2 t = ivec2((v_glyph % u_soft_columns) * u_soft_slot + x, (v_glyph / u_soft_columns) * u_soft_slot + y);
        return texelFetch(u_soft_atlas, t, 0).r;
    }
    ivec2 t = ivec2((v_glyph % u_atlas_columns) * u_slot + x, (v_glyph / u_atlas_columns) * u_slot + y);
    return texelFetch(u_atlas, t, 0).r;
}

float lit(vec2 d) {
    int x = int(floor(d.x));
    int y = int(floor(d.y));
    float v = dot_at(x, y);
    if (u_stretch && fract(d.x) < 0.5) v = max(v, dot_at(x - 1, y));
    return v;
}

void main() {
    // Position in the cell from the pixel itself rather than an interpolated
    // corner: the two triangles of a quad interpolate slightly differently,
    // which showed as a step half-way along lines.
    vec2 v_local = (vec2(gl_FragCoord.x, u_viewport.y - gl_FragCoord.y) - v_origin) / v_size;
    vec2 cell = vec2(v_matrix);
    vec2 d = v_local * cell;
    if ((v_flags & 2) != 0) d.y = v_local.y * cell.y * 0.5;
    if ((v_flags & 4) != 0) d.y = (1.0 + v_local.y) * cell.y * 0.5;

    // Box-filter each pixel: six samples across and two down. In 132-column
    // mode a dot is narrower than a pixel, and fewer samples would drop
    // whole dot columns, giving strokes of uneven weight.
    // Samples stay inside the cell: one falling past its edge would darken
    // the edge pixels and break lines that run on into the next cell.
    vec2 dx = dFdx(d);
    vec2 dy = dFdy(d) * 0.25;
    vec2 last = cell - vec2(0.001);
    float coverage = 0.0;
    for (int i = 0; i < 6; ++i) {
        vec2 offset = dx * ((float(i) + 0.5) / 6.0 - 0.5);
        coverage += lit(clamp(d + offset - dy, vec2(0.0), last))
            + lit(clamp(d + offset + dy, vec2(0.0), last));
    }
    coverage /= 12.0;

    if ((v_flags & 32) != 0 || (v_flags & 8) != 0) coverage = 0.0;
    if ((v_flags & 1) != 0 && int(floor(d.y)) == v_matrix.y - 1) coverage = 1.0;
    if ((v_flags & 512) != 0 && int(floor(d.y)) >= v_matrix.y - 1) coverage = 1.0 - coverage;

    // Scan lines are fixed per screen line (two per dot row on a VT220, one
    // on a VT420); darken the gap in the lower half of each.
    float scan = fract(v_local.y * u_cell_scan_lines);
    float beam = 1.0 - u_scanlines * smoothstep(0.55, 0.95, scan);

    vec3 fg = v_fg;
    vec3 bg = v_bg;
    // Cursor and selection each reverse the cell; both together cancel out.
    if ((((v_flags & 16) != 0) ? 1 : 0) + (((v_flags & 128) != 0) ? 1 : 0) == 1) { vec3 t = fg; fg = bg; bg = t; }
    vec3 color = mix(bg, fg * beam, coverage);

    if ((v_flags & 64) != 0) {
        vec2 px = v_local * v_size;
        if (px.x < 1.5 || px.y < 1.5 || px.x > v_size.x - 1.5 || px.y > v_size.y - 1.5) color = v_fg;
    }
    o_color = vec4(color, 1.0);
}
"#;

impl Renderer {
    /// Compiles shaders and uploads the glyph atlas of every face in `fonts`.
    ///
    /// # Safety
    /// A GL context must be current and remain current for every later call.
    pub unsafe fn new(gl: &glow::Context, fonts: FontSet) -> Result<Renderer, String> {
        unsafe {
            let header = if gl.version().is_embedded {
                "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp sampler2D;\n"
            } else {
                "#version 330 core\n"
            };
            let program = link(
                gl,
                &format!("{header}{VERTEX}"),
                &format!("{header}{FRAGMENT}"),
            )?;

            let vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vao));

            let quad = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(quad));
            let corners: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&corners), glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);

            let instances = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(instances));
            let stride = (INSTANCE_LEN * 4) as i32;
            for (loc, size, offset) in [(1u32, 4, 0), (2, 2, 4), (3, 3, 6), (4, 3, 9), (5, 2, 12)] {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(loc, size, glow::FLOAT, false, stride, offset * 4);
                gl.vertex_attrib_divisor(loc, 1);
            }
            gl.bind_vertex_array(None);

            let atlas = upload_atlas(gl, &fonts)?;
            let soft = SoftAtlas::default();
            let soft_texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(soft_texture));
            set_nearest(gl);
            let uniforms = Uniforms {
                viewport: gl.get_uniform_location(program, "u_viewport"),
                atlas: gl.get_uniform_location(program, "u_atlas"),
                soft_atlas: gl.get_uniform_location(program, "u_soft_atlas"),
                slot: gl.get_uniform_location(program, "u_slot"),
                cell_scan_lines: gl.get_uniform_location(program, "u_cell_scan_lines"),
                soft_slot: gl.get_uniform_location(program, "u_soft_slot"),
                soft_columns: gl.get_uniform_location(program, "u_soft_columns"),
                atlas_columns: gl.get_uniform_location(program, "u_atlas_columns"),
                scanlines: gl.get_uniform_location(program, "u_scanlines"),
                stretch: gl.get_uniform_location(program, "u_stretch"),
            };
            Ok(Renderer {
                program,
                vao,
                quad,
                instances,
                atlas,
                soft_texture,
                soft,
                fonts,
                scratch: Vec::new(),
                uniforms,
            })
        }
    }

    pub fn fonts(&self) -> &FontSet {
        &self.fonts
    }

    /// Draws one frame into the currently bound framebuffer. `indicator` is
    /// the text shown when the indicator status line is selected.
    ///
    /// # Safety
    /// The context passed to [`Renderer::new`] must be current.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn draw(
        &mut self,
        gl: &glow::Context,
        term: &Terminal,
        layout: &Layout,
        viewport: (u32, u32),
        frame: FrameState,
        theme: &Theme,
        indicator: &str,
    ) {
        build_instances(
            term,
            layout,
            frame,
            theme,
            &self.fonts,
            &mut self.soft,
            indicator,
            &mut self.scratch,
        );
        unsafe {
            gl.viewport(0, 0, viewport.0 as i32, viewport.1 as i32);
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.clear_color(theme.bezel[0], theme.bezel[1], theme.bezel[2], 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            if self.soft.dirty {
                gl.bind_texture(glow::TEXTURE_2D, Some(self.soft_texture));
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::R8 as i32,
                    SoftAtlas::WIDTH as i32,
                    SoftAtlas::HEIGHT as i32,
                    0,
                    glow::RED,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&self.soft.pixels)),
                );
                self.soft.dirty = false;
            }

            gl.use_program(Some(self.program));
            let u = &self.uniforms;
            gl.uniform_2_f32(u.viewport.as_ref(), viewport.0 as f32, viewport.1 as f32);
            gl.uniform_1_i32(u.atlas.as_ref(), 0);
            gl.uniform_1_i32(u.soft_atlas.as_ref(), 1);
            gl.uniform_1_i32(u.slot.as_ref(), ATLAS_SLOT);
            let face = self.fonts.face(term.modes().columns_132, layout.rows);
            let scan_lines =
                f32::from(face.height) * f32::from(self.fonts.family().scan_lines_per_dot());
            gl.uniform_1_f32(u.cell_scan_lines.as_ref(), scan_lines);
            gl.uniform_1_i32(u.atlas_columns.as_ref(), ATLAS_COLUMNS);
            gl.uniform_1_i32(u.soft_slot.as_ref(), SOFT_SLOT as i32);
            gl.uniform_1_i32(u.soft_columns.as_ref(), SOFT_ATLAS_COLUMNS as i32);
            gl.uniform_1_f32(u.scanlines.as_ref(), theme.scanlines);
            gl.uniform_1_i32(u.stretch.as_ref(), i32::from(theme.dot_stretch));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.soft_texture));
            gl.active_texture(glow::TEXTURE0);

            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instances));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                as_bytes(&self.scratch),
                glow::STREAM_DRAW,
            );
            let count = (self.scratch.len() / INSTANCE_LEN) as i32;
            gl.draw_arrays_instanced(glow::TRIANGLES, 0, 6, count);
            gl.bind_vertex_array(None);
        }
    }

    /// Releases GL objects.
    ///
    /// # Safety
    /// The context passed to [`Renderer::new`] must be current.
    pub unsafe fn destroy(self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.quad);
            gl.delete_buffer(self.instances);
            gl.delete_texture(self.atlas);
            gl.delete_texture(self.soft_texture);
        }
    }
}

unsafe fn set_nearest(gl: &glow::Context) {
    for (param, value) in [
        (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
        (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
    ] {
        unsafe { gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32) };
    }
}

fn as_bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding or invalid bit patterns; u8 has alignment 1.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}

unsafe fn link(gl: &glow::Context, vs: &str, fs: &str) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, src);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!(
                    "shader compile failed: {}",
                    gl.get_shader_info_log(shader)
                ));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            return Err(format!(
                "shader link failed: {}",
                gl.get_program_info_log(program)
            ));
        }
        Ok(program)
    }
}

/// Uploads every face's glyphs, face after face, in 16×16 slots.
unsafe fn upload_atlas(gl: &glow::Context, fonts: &FontSet) -> Result<glow::Texture, String> {
    let glyphs: Vec<&vt_fonts::Glyph> = fonts
        .faces()
        .iter()
        .flat_map(|face| face.glyphs().iter())
        .collect();
    let count = glyphs.len() as i32;
    let rows = (count + ATLAS_COLUMNS - 1) / ATLAS_COLUMNS;
    let (w, h) = (ATLAS_COLUMNS * ATLAS_SLOT, rows * ATLAS_SLOT);
    let mut pixels = vec![0u8; (w * h) as usize];
    for (i, glyph) in glyphs.into_iter().enumerate() {
        let (gx, gy) = (
            i as i32 % ATLAS_COLUMNS * ATLAS_SLOT,
            i as i32 / ATLAS_COLUMNS * ATLAS_SLOT,
        );
        for y in 0..ATLAS_SLOT {
            for x in 0..ATLAS_SLOT {
                if glyph.dot(x as usize, y as usize) {
                    pixels[((gy + y) * w + gx + x) as usize] = 255;
                }
            }
        }
    }
    unsafe {
        let tex = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::R8 as i32,
            w,
            h,
            0,
            glow::RED,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&pixels)),
        );
        set_nearest(gl);
        Ok(tex)
    }
}
