use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use egui::{Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use egui_wgpu::{Callback, CallbackResources, CallbackTrait, ScreenDescriptor};
use wgpu::util::DeviceExt;

use crate::model::{Emu, SlideSize};

const EMU_PER_INCH: f32 = 914_400.0;
const POINTS_PER_INCH: f32 = 72.0;
const GRID_STEP_EMU: i64 = 457_200;
const RULER_WIDTH: f32 = 25.0;
const RULER_HEIGHT: f32 = 24.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuideAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Guide {
    axis: GuideAxis,
    position: Emu,
}

/// UI-only viewport state. Guide positions stay in EMU; pan is screen-space state.
pub struct CanvasView {
    zoom_percent: u16,
    pan: Vec2,
    show_rulers: bool,
    show_grid: bool,
    snap_to_grid: bool,
    guides: Vec<Guide>,
    dragged_guide: Option<usize>,
    last_viewport_size: Vec2,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self {
            zoom_percent: 75,
            pan: Vec2::ZERO,
            show_rulers: true,
            show_grid: false,
            snap_to_grid: true,
            guides: Vec::new(),
            dragged_guide: None,
            last_viewport_size: Vec2::ZERO,
        }
    }
}

impl CanvasView {
    pub fn zoom_percent(&self) -> u16 {
        self.zoom_percent
    }

    pub fn set_zoom_percent(&mut self, zoom_percent: u16) {
        self.zoom_percent = zoom_percent.clamp(10, 400);
    }

    pub fn fit_to_slide(&mut self, slide_size: SlideSize) {
        if self.last_viewport_size.x <= 0.0 || self.last_viewport_size.y <= 0.0 {
            return;
        }
        let width = emu_to_points(slide_size.width) + 48.0;
        let height = emu_to_points(slide_size.height) + 48.0;
        let fit = (self.last_viewport_size.x / width)
            .min(self.last_viewport_size.y / height)
            .clamp(0.1, 4.0);
        self.set_zoom_percent((fit * 100.0).round() as u16);
        self.pan = Vec2::ZERO;
    }

    pub fn rulers_visible(&self) -> bool {
        self.show_rulers
    }

    pub fn set_rulers_visible(&mut self, visible: bool) {
        self.show_rulers = visible;
    }

    pub fn grid_visible(&self) -> bool {
        self.show_grid
    }

    pub fn set_grid_visible(&mut self, visible: bool) {
        self.show_grid = visible;
    }

    pub fn snap_to_grid(&self) -> bool {
        self.snap_to_grid
    }

    pub fn set_snap_to_grid(&mut self, enabled: bool) {
        self.snap_to_grid = enabled;
    }

    pub fn guide_count(&self) -> usize {
        self.guides.len()
    }

    pub fn clear_guides(&mut self) {
        self.guides.clear();
        self.dragged_guide = None;
    }

    pub fn show(&mut self, ui: &mut Ui, renderer: &CanvasRenderer, slide_size: SlideSize) {
        let available = ui.available_size().max(Vec2::ZERO);
        let (outer, _) = ui.allocate_exact_size(available, Sense::hover());
        if outer.width() < 80.0 || outer.height() < 80.0 {
            return;
        }

        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, 0.0, Color32::from_rgb(231, 234, 238));

        let content = if self.show_rulers {
            Rect::from_min_max(outer.min + Vec2::new(RULER_WIDTH, RULER_HEIGHT), outer.max)
        } else {
            outer
        };
        self.last_viewport_size = content.size();

        let pixels_per_emu = POINTS_PER_INCH * f32::from(self.zoom_percent) / 100.0 / EMU_PER_INCH;
        let slide_size_points = Vec2::new(
            slide_size.width.0 as f32 * pixels_per_emu,
            slide_size.height.0 as f32 * pixels_per_emu,
        );
        let response = ui.interact(
            content,
            ui.scope_id().with("presentation-canvas"),
            Sense::click_and_drag(),
        );
        self.update_viewport_input(ui, &response);
        let slide_rect = Rect::from_center_size(content.center() + self.pan, slide_size_points);

        let top_ruler = self.show_rulers.then(|| {
            Rect::from_min_max(
                Pos2::new(content.left(), outer.top()),
                Pos2::new(outer.right(), content.top()),
            )
        });
        let left_ruler = self.show_rulers.then(|| {
            Rect::from_min_max(
                Pos2::new(outer.left(), content.top()),
                Pos2::new(content.left(), outer.bottom()),
            )
        });

        if let (Some(top), Some(left)) = (top_ruler, left_ruler) {
            painter.rect_filled(
                Rect::from_min_max(outer.min, Pos2::new(content.left(), content.top())),
                0.0,
                Color32::from_rgb(247, 248, 250),
            );
            painter.rect_filled(top, 0.0, Color32::from_rgb(247, 248, 250));
            painter.rect_filled(left, 0.0, Color32::from_rgb(247, 248, 250));
            painter.line_segment(
                [top.left_bottom(), top.right_bottom()],
                Stroke::new(1.0, Color32::from_rgb(195, 199, 205)),
            );
            painter.line_segment(
                [left.right_top(), left.right_bottom()],
                Stroke::new(1.0, Color32::from_rgb(195, 199, 205)),
            );
            self.draw_rulers(&painter, top, left, slide_rect, slide_size);
            self.handle_guide_drag(
                ui,
                ui.scope_id().with("top-ruler"),
                top,
                GuideAxis::Vertical,
                slide_rect,
                slide_size,
            );
            self.handle_guide_drag(
                ui,
                ui.scope_id().with("left-ruler"),
                left,
                GuideAxis::Horizontal,
                slide_rect,
                slide_size,
            );
        }

        let mut vertices = Vec::with_capacity(6 + 6 * 4 + 6 * 48);
        append_rect(
            &mut vertices,
            content,
            slide_rect.translate(Vec2::new(3.0, 4.0)).expand(1.0),
            Color32::from_rgba_unmultiplied(40, 48, 60, 45),
        );
        append_rect(&mut vertices, content, slide_rect, Color32::WHITE);
        append_slide_border(&mut vertices, content, slide_rect);
        if self.show_grid {
            append_grid(
                &mut vertices,
                content,
                slide_rect,
                slide_size,
                pixels_per_emu,
            );
        }
        ui.painter().add(Callback::new_paint_callback(
            content,
            CanvasPaintCallback::new(Arc::clone(&renderer.pipeline), vertices),
        ));

        self.draw_guides(&painter, content, slide_rect, pixels_per_emu);
        if let Some(pointer) = response.hover_pos() {
            let cursor_emu = point_to_emu(pointer.x, slide_rect, pixels_per_emu);
            if cursor_emu.0 >= 0 && cursor_emu.0 <= slide_size.width.0 {
                ui.output_mut(|output| {
                    output.cursor_icon = egui::CursorIcon::Crosshair;
                });
            }
        }
    }

    fn update_viewport_input(&mut self, ui: &Ui, response: &Response) {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if response.hovered() && scroll.abs() > f32::EPSILON {
            let factor = (1.0 + scroll * 0.001).clamp(0.8, 1.2);
            self.set_zoom_percent((f32::from(self.zoom_percent) * factor).round() as u16);
        }

        let space_pan = ui.input(|input| input.key_down(egui::Key::Space));
        let middle_pan = ui.input(|input| {
            input.pointer.button_down(egui::PointerButton::Middle)
                && input
                    .pointer
                    .hover_pos()
                    .is_some_and(|point| response.rect.contains(point))
        });
        if middle_pan || (space_pan && response.dragged_by(egui::PointerButton::Primary)) {
            self.pan += ui.input(|input| input.pointer.delta());
        }
    }

    fn draw_rulers(
        &self,
        painter: &egui::Painter,
        horizontal: Rect,
        vertical: Rect,
        slide: Rect,
        slide_size: SlideSize,
    ) {
        let scale = POINTS_PER_INCH * f32::from(self.zoom_percent) / 100.0;
        draw_one_ruler(
            painter,
            horizontal,
            slide.left(),
            slide.right(),
            scale,
            slide_size.width.0,
            true,
        );
        draw_one_ruler(
            painter,
            vertical,
            slide.top(),
            slide.bottom(),
            scale,
            slide_size.height.0,
            false,
        );
    }

    fn draw_guides(
        &self,
        painter: &egui::Painter,
        viewport: Rect,
        slide: Rect,
        pixels_per_emu: f32,
    ) {
        let guide_color = Color32::from_rgb(52, 152, 219);
        for guide in &self.guides {
            let offset = guide.position.0 as f32 * pixels_per_emu;
            match guide.axis {
                GuideAxis::Vertical => {
                    let x = slide.left() + offset;
                    painter.line_segment(
                        [
                            Pos2::new(x, viewport.top()),
                            Pos2::new(x, viewport.bottom()),
                        ],
                        Stroke::new(1.0, guide_color),
                    );
                }
                GuideAxis::Horizontal => {
                    let y = slide.top() + offset;
                    painter.line_segment(
                        [
                            Pos2::new(viewport.left(), y),
                            Pos2::new(viewport.right(), y),
                        ],
                        Stroke::new(1.0, guide_color),
                    );
                }
            }
        }
    }

    fn handle_guide_drag(
        &mut self,
        ui: &mut Ui,
        id: egui::Id,
        ruler: Rect,
        axis: GuideAxis,
        slide_rect: Rect,
        slide_size: SlideSize,
    ) {
        let response = ui.interact(ruler, id, Sense::drag());
        if response.drag_started()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let position = self.guide_position(axis, pointer, slide_rect, slide_size, None);
            self.guides.push(Guide { axis, position });
            self.dragged_guide = Some(self.guides.len() - 1);
        }
        if response.dragged()
            && let (Some(pointer), Some(index)) =
                (response.interact_pointer_pos(), self.dragged_guide)
        {
            let position = self.guide_position(axis, pointer, slide_rect, slide_size, Some(index));
            if let Some(guide) = self.guides.get_mut(index) {
                guide.position = position;
            }
        }
        if response.drag_stopped() {
            self.dragged_guide = None;
        }
    }

    fn guide_position(
        &self,
        axis: GuideAxis,
        pointer: Pos2,
        slide: Rect,
        slide_size: SlideSize,
        skip: Option<usize>,
    ) -> Emu {
        let extent = match axis {
            GuideAxis::Horizontal => slide_size.height.0,
            GuideAxis::Vertical => slide_size.width.0,
        };
        let raw = match axis {
            GuideAxis::Horizontal => point_to_emu_y(
                pointer.y,
                slide,
                POINTS_PER_INCH * f32::from(self.zoom_percent) / 100.0,
            ),
            GuideAxis::Vertical => point_to_emu_x(
                pointer.x,
                slide,
                POINTS_PER_INCH * f32::from(self.zoom_percent) / 100.0,
            ),
        }
        .clamp(0, extent);
        let mut position = raw;
        if self.snap_to_grid {
            position = snap_to_grid(position);
        }
        let proximity =
            (12.0 / (POINTS_PER_INCH * f32::from(self.zoom_percent) / 100.0) * EMU_PER_INCH) as i64;
        if let Some(guide) = self
            .guides
            .iter()
            .enumerate()
            .filter(|(index, guide)| Some(*index) != skip && guide.axis == axis)
            .map(|(_, guide)| guide.position.0)
            .min_by_key(|candidate| (candidate - position).abs())
            .filter(|candidate| (candidate - position).abs() <= proximity)
        {
            position = guide;
        }
        Emu(position.clamp(0, extent))
    }
}

/// Owns the GPU pipeline for slide content. egui paints the shell and overlays only.
pub struct CanvasRenderer {
    pipeline: Arc<wgpu::RenderPipeline>,
}

impl CanvasRenderer {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("canvas.wgsl"));
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("presentationpoint_slide_scene"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<CanvasVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count.max(1),
                ..wgpu::MultisampleState::default()
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline: Arc::new(pipeline),
        }
    }
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct CanvasVertex {
    position: [f32; 2],
    color: [f32; 4],
}

struct CanvasPaintCallback {
    pipeline: Arc<wgpu::RenderPipeline>,
    vertices: Vec<CanvasVertex>,
    vertex_buffer: Mutex<Option<wgpu::Buffer>>,
}

impl CanvasPaintCallback {
    fn new(pipeline: Arc<wgpu::RenderPipeline>, vertices: Vec<CanvasVertex>) -> Self {
        Self {
            pipeline,
            vertices,
            vertex_buffer: Mutex::new(None),
        }
    }
}

impl CallbackTrait for CanvasPaintCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("presentationpoint_slide_vertices"),
            contents: bytemuck::cast_slice(&self.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        if let Ok(mut buffer) = self.vertex_buffer.lock() {
            *buffer = Some(vertex_buffer);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &CallbackResources,
    ) {
        let Ok(buffer) = self.vertex_buffer.lock() else {
            return;
        };
        let Some(vertex_buffer) = buffer.as_ref() else {
            return;
        };
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.draw(0..self.vertices.len() as u32, 0..1);
    }
}

fn append_slide_border(vertices: &mut Vec<CanvasVertex>, viewport: Rect, slide: Rect) {
    let color = Color32::from_rgb(180, 184, 190);
    append_rect(
        vertices,
        viewport,
        Rect::from_min_max(slide.min, Pos2::new(slide.right(), slide.top() + 1.0)),
        color,
    );
    append_rect(
        vertices,
        viewport,
        Rect::from_min_max(Pos2::new(slide.left(), slide.bottom() - 1.0), slide.max),
        color,
    );
    append_rect(
        vertices,
        viewport,
        Rect::from_min_max(slide.min, Pos2::new(slide.left() + 1.0, slide.bottom())),
        color,
    );
    append_rect(
        vertices,
        viewport,
        Rect::from_min_max(Pos2::new(slide.right() - 1.0, slide.top()), slide.max),
        color,
    );
}

fn append_grid(
    vertices: &mut Vec<CanvasVertex>,
    viewport: Rect,
    slide: Rect,
    slide_size: SlideSize,
    pixels_per_emu: f32,
) {
    let color = Color32::from_rgba_unmultiplied(225, 230, 237, 190);
    let width = slide_size.width.0;
    let height = slide_size.height.0;
    let step = GRID_STEP_EMU;
    let line_width = 0.65;
    let mut coordinate = step;
    while coordinate < width {
        let x = slide.left() + coordinate as f32 * pixels_per_emu;
        append_rect(
            vertices,
            viewport,
            Rect::from_min_max(
                Pos2::new(x - line_width / 2.0, slide.top()),
                Pos2::new(x + line_width / 2.0, slide.bottom()),
            ),
            color,
        );
        coordinate += step;
    }
    coordinate = step;
    while coordinate < height {
        let y = slide.top() + coordinate as f32 * pixels_per_emu;
        append_rect(
            vertices,
            viewport,
            Rect::from_min_max(
                Pos2::new(slide.left(), y - line_width / 2.0),
                Pos2::new(slide.right(), y + line_width / 2.0),
            ),
            color,
        );
        coordinate += step;
    }
}

fn append_rect(vertices: &mut Vec<CanvasVertex>, viewport: Rect, rect: Rect, color: Color32) {
    let min = to_ndc(viewport, rect.min);
    let max = to_ndc(viewport, rect.max);
    let top_left = [min[0], min[1]];
    let top_right = [max[0], min[1]];
    let bottom_right = [max[0], max[1]];
    let bottom_left = [min[0], max[1]];
    let rgba = color.to_array().map(|channel| f32::from(channel) / 255.0);
    for position in [
        top_left,
        bottom_left,
        bottom_right,
        top_left,
        bottom_right,
        top_right,
    ] {
        vertices.push(CanvasVertex {
            position,
            color: rgba,
        });
    }
}

fn to_ndc(viewport: Rect, point: Pos2) -> [f32; 2] {
    let width = viewport.width().max(f32::EPSILON);
    let height = viewport.height().max(f32::EPSILON);
    [
        ((point.x - viewport.left()) / width) * 2.0 - 1.0,
        1.0 - ((point.y - viewport.top()) / height) * 2.0,
    ]
}

fn draw_one_ruler(
    painter: &egui::Painter,
    ruler: Rect,
    slide_start: f32,
    slide_end: f32,
    points_per_inch: f32,
    extent_emu: i64,
    horizontal: bool,
) {
    let major_inches = if points_per_inch >= 48.0 {
        1
    } else if points_per_inch >= 24.0 {
        2
    } else if points_per_inch >= 12.0 {
        4
    } else {
        8
    };
    let max_inches = extent_emu / 914_400;
    let step_points = points_per_inch * major_inches as f32;
    let mut inch = 0_i64;
    while inch <= max_inches {
        let coordinate = slide_start + inch as f32 * points_per_inch;
        if coordinate >= slide_start - step_points && coordinate <= slide_end + step_points {
            let major = inch % i64::from(major_inches) == 0;
            let tick = if major { 8.0 } else { 4.0 };
            let (from, to, label_at) = if horizontal {
                (
                    Pos2::new(coordinate, ruler.bottom() - tick),
                    Pos2::new(coordinate, ruler.bottom()),
                    Pos2::new(coordinate + 2.0, ruler.top() + 2.0),
                )
            } else {
                (
                    Pos2::new(ruler.right() - tick, coordinate),
                    Pos2::new(ruler.right(), coordinate),
                    Pos2::new(ruler.left() + 2.0, coordinate + 1.0),
                )
            };
            painter.line_segment(
                [from, to],
                Stroke::new(1.0, Color32::from_rgb(123, 128, 135)),
            );
            if major {
                painter.text(
                    label_at,
                    Align2::LEFT_TOP,
                    inch.to_string(),
                    FontId::monospace(9.0),
                    Color32::from_rgb(86, 90, 96),
                );
            }
        }
        inch += 1;
    }
}

fn point_to_emu(x: f32, slide: Rect, pixels_per_emu: f32) -> Emu {
    Emu(((x - slide.left()) / pixels_per_emu).round() as i64)
}

fn point_to_emu_x(x: f32, slide: Rect, points_per_inch: f32) -> i64 {
    (((x - slide.left()) / points_per_inch) * EMU_PER_INCH).round() as i64
}

fn point_to_emu_y(y: f32, slide: Rect, points_per_inch: f32) -> i64 {
    (((y - slide.top()) / points_per_inch) * EMU_PER_INCH).round() as i64
}

fn emu_to_points(emu: Emu) -> f32 {
    emu.0 as f32 / EMU_PER_INCH * POINTS_PER_INCH
}

fn snap_to_grid(position: i64) -> i64 {
    ((position + GRID_STEP_EMU / 2) / GRID_STEP_EMU) * GRID_STEP_EMU
}

#[cfg(test)]
mod tests {
    use super::{EMU_PER_INCH, point_to_emu, snap_to_grid};
    use crate::model::Emu;
    use egui::{Pos2, Rect, vec2};

    #[test]
    fn guide_grid_snap_uses_half_inch_emu_steps() {
        assert_eq!(snap_to_grid(Emu::inches(1).0 + 100_000), Emu::inches(1).0);
        assert_eq!(snap_to_grid(Emu::inches(1).0 + 300_000), Emu(1_371_600).0);
    }

    #[test]
    fn canvas_pointer_conversion_returns_integer_emu() {
        let slide = Rect::from_min_size(Pos2::ZERO, vec2(960.0, 540.0));
        let scale = 72.0 / EMU_PER_INCH;
        assert_eq!(point_to_emu(72.0, slide, scale), Emu::inches(1));
    }
}
