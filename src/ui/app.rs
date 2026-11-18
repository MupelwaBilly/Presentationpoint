use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;

use egui::{Context, ViewportId};
use egui_wgpu::winit::Painter;
use egui_wgpu::{RendererOptions, WgpuConfiguration};
use egui_winit::State as EguiWinitState;
use futures::executor::block_on;
use rfd::FileDialog;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::AppError;
use crate::canvas::{CanvasRenderer, CanvasView};
use crate::commands::{CommandBus, ReplaceState};
use crate::model::SlideSize;
use crate::ooxml::PptxDocument;
use crate::storage::{AppStorage, Settings};
use crate::ui::tokens;

struct UiState {
    current_path: Option<PathBuf>,
    recent_paths: Vec<PathBuf>,
    status: String,
    settings: Settings,
    backstage: Option<BackstagePage>,
    active_tab: RibbonTab,
    selected_slide: usize,
    task_pane: Option<TaskPane>,
    show_notes: bool,
    canvas: CanvasView,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum BackstagePage {
    Info,
    New,
    Open,
    Recent,
    Save,
    SaveAs,
    SaveCopy,
    Print,
    Export,
    Reveal,
    Options,
    About,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RibbonTab {
    Home,
    Insert,
    Design,
    Transitions,
    Animations,
    SlideShow,
    Review,
    View,
}

impl RibbonTab {
    const ALL: [Self; 8] = [
        Self::Home,
        Self::Insert,
        Self::Design,
        Self::Transitions,
        Self::Animations,
        Self::SlideShow,
        Self::Review,
        Self::View,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Insert => "Insert",
            Self::Design => "Design",
            Self::Transitions => "Transitions",
            Self::Animations => "Animations",
            Self::SlideShow => "Slide Show",
            Self::Review => "Review",
            Self::View => "View",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TaskPane {
    FormatShape,
    FormatPicture,
    Selection,
    AltText,
}

impl TaskPane {
    const fn label(self) -> &'static str {
        match self {
            Self::FormatShape => "Format Shape",
            Self::FormatPicture => "Format Picture",
            Self::Selection => "Selection",
            Self::AltText => "Alt Text",
        }
    }
}

pub struct PresentationpointApp {
    context: Context,
    window: Option<Arc<Window>>,
    egui_winit: Option<EguiWinitState>,
    painter: Option<Painter>,
    canvas_renderer: Option<CanvasRenderer>,
    document: CommandBus<Option<PptxDocument>>,
    storage: AppStorage,
    ui: UiState,
    startup_error: Option<AppError>,
}

impl PresentationpointApp {
    fn new(storage: AppStorage) -> Result<Self, AppError> {
        crate::ui::icons::validate_bundled_icons()?;
        let settings = storage.settings()?;
        storage.save_settings(&settings)?;
        let recent_paths = storage.recent_files()?;
        let document = PptxDocument::new_minimal(SlideSize::WIDESCREEN)?;
        let context = Context::default();
        tokens::apply_office_chrome(&context);
        context.set_zoom_factor(f32::from(settings.ui_scale_percent) / 100.0);

        Ok(Self {
            context,
            window: None,
            egui_winit: None,
            painter: None,
            canvas_renderer: None,
            document: CommandBus::new(Some(document)),
            storage,
            ui: UiState {
                current_path: None,
                recent_paths,
                status: "Blank presentation".to_owned(),
                settings,
                backstage: None,
                active_tab: RibbonTab::Home,
                selected_slide: 0,
                task_pane: None,
                show_notes: false,
                canvas: CanvasView::default(),
            },
            startup_error: None,
        })
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title("Presentationpoint")
            .with_inner_size(LogicalSize::new(1920.0, 1080.0))
            .with_min_inner_size(LogicalSize::new(1024.0, 700.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.startup_error =
                    Some(AppError::Gpu(format!("window creation failed: {error}")));
                event_loop.exit();
                return;
            }
        };
        let state = EguiWinitState::new(
            self.context.clone(),
            ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let mut painter = block_on(Painter::new(
            self.context.clone(),
            WgpuConfiguration::default(),
            false,
            RendererOptions {
                msaa_samples: 4,
                ..RendererOptions::default()
            },
        ));
        if let Err(error) =
            block_on(painter.set_window(ViewportId::ROOT, Some(Arc::clone(&window))))
        {
            self.startup_error = Some(AppError::Gpu(error.to_string()));
            event_loop.exit();
            return;
        }

        let Some(render_state) = painter.render_state() else {
            self.startup_error = Some(AppError::Gpu(
                "the window renderer did not expose its WGPU device".to_owned(),
            ));
            event_loop.exit();
            return;
        };
        let canvas_renderer =
            CanvasRenderer::new(&render_state.device, render_state.target_format, 4);

        self.window = Some(Arc::clone(&window));
        self.egui_winit = Some(state);
        self.painter = Some(painter);
        self.canvas_renderer = Some(canvas_renderer);
        window.request_redraw();
    }

    fn draw(&mut self) {
        let Some(window) = self.window.as_ref().map(Arc::clone) else {
            return;
        };
        let Some(egui_winit) = self.egui_winit.as_mut() else {
            return;
        };

        let input = egui_winit.take_egui_input(&window);
        let storage = &self.storage;
        let document = &mut self.document;
        let state = &mut self.ui;
        let canvas_renderer = self.canvas_renderer.as_ref();
        let mut output = self.context.run_ui(input, |root_ui| {
            draw_main_window(
                root_ui,
                &self.context,
                state,
                document,
                storage,
                canvas_renderer,
            );
        });
        egui_winit.handle_platform_output(&window, output.platform_output);

        let clipped_primitives = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        if let Some(painter) = self.painter.as_mut() {
            let _ = painter.paint_and_update_textures(
                ViewportId::ROOT,
                output.pixels_per_point,
                [0.95, 0.96, 0.97, 1.0],
                &clipped_primitives,
                &mut output.textures_delta,
                Vec::new(),
                &window,
            );
        }
    }

    fn resize_painter(&mut self, size: PhysicalSize<u32>) {
        let (Some(width), Some(height), Some(painter)) = (
            NonZeroU32::new(size.width),
            NonZeroU32::new(size.height),
            self.painter.as_mut(),
        ) else {
            return;
        };
        painter.on_window_resized(ViewportId::ROOT, width, height);
    }
}

impl ApplicationHandler for PresentationpointApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.create_window(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref().map(Arc::clone) else {
            return;
        };
        let response = self
            .egui_winit
            .as_mut()
            .map(|state| state.on_window_event(&window, &event));

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.resize_painter(size);
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {
                if response.is_some_and(|response| response.repaint) {
                    window.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn draw_main_window(
    root_ui: &mut egui::Ui,
    context: &Context,
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
    canvas_renderer: Option<&CanvasRenderer>,
) {
    let shortcuts = root_ui.ctx().input(|input| {
        let command = input.modifiers.command;
        (
            command && input.key_pressed(egui::Key::S),
            command && input.key_pressed(egui::Key::O),
            command && !input.modifiers.shift && input.key_pressed(egui::Key::Z),
            command
                && (input.key_pressed(egui::Key::Y)
                    || (input.modifiers.shift && input.key_pressed(egui::Key::Z))),
        )
    });
    if shortcuts.0 {
        save_current(state, document_bus, storage);
    }
    if shortcuts.1 {
        choose_and_open(state, document_bus, storage);
    }
    if shortcuts.2 {
        state.status = match document_bus.undo() {
            Ok(true) => "Undid the last document command".to_owned(),
            Ok(false) => "Nothing to undo".to_owned(),
            Err(error) => error.to_string(),
        };
    }
    if shortcuts.3 {
        state.status = match document_bus.redo() {
            Ok(true) => "Redid the last document command".to_owned(),
            Ok(false) => "Nothing to redo".to_owned(),
            Err(error) => error.to_string(),
        };
    }

    egui::Panel::top("title_bar")
        .exact_size(38.0)
        .show(root_ui, |ui| {
            ui.horizontal_centered(|ui| {
                if ui.button("File").clicked() {
                    state.backstage = Some(BackstagePage::Info);
                }
                ui.separator();
                if ui.button("Save").on_hover_text("Save (Ctrl+S)").clicked() {
                    save_current(state, document_bus, storage);
                }
                let can_undo = document_bus.undo_depth() > 0;
                if ui
                    .add_enabled(can_undo, egui::Button::new("Undo"))
                    .on_hover_text("Undo (Ctrl+Z)")
                    .clicked()
                {
                    state.status = match document_bus.undo() {
                        Ok(true) => "Undid the last document command".to_owned(),
                        Ok(false) => "Nothing to undo".to_owned(),
                        Err(error) => error.to_string(),
                    };
                }
                let can_redo = document_bus.redo_depth() > 0;
                if ui
                    .add_enabled(can_redo, egui::Button::new("Redo"))
                    .on_hover_text("Redo (Ctrl+Shift+Z)")
                    .clicked()
                {
                    state.status = match document_bus.redo() {
                        Ok(true) => "Redid the last document command".to_owned(),
                        Ok(false) => "Nothing to redo".to_owned(),
                        Err(error) => error.to_string(),
                    };
                }
                ui.separator();
                let file_name = state
                    .current_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .and_then(|name| name.to_str())
                    .unwrap_or("Presentationpoint");
                ui.label(file_name);
                if state.backstage.is_some() {
                    ui.separator();
                    if ui.button("Back to presentation").clicked() {
                        state.backstage = None;
                    }
                }
            });
        });

    if state.backstage.is_some() {
        draw_backstage(root_ui, context, state, document_bus, storage);
        return;
    }

    egui::Panel::top("ribbon_tabs")
        .exact_size(34.0)
        .show(root_ui, |ui| {
            ui.horizontal_centered(|ui| {
                for tab in RibbonTab::ALL {
                    if ui
                        .selectable_label(state.active_tab == tab, tab.label())
                        .clicked()
                    {
                        state.active_tab = tab;
                    }
                }
                ui.separator();
                for label in [
                    "Shape Format",
                    "Picture Format",
                    "Table Design",
                    "Chart Design",
                ] {
                    ui.add_enabled(false, egui::Button::new(label))
                        .on_hover_text(
                            "Select a matching slide object to use this contextual tab.",
                        );
                }
            });
        });

    egui::Panel::top("ribbon_commands")
        .exact_size(96.0)
        .show(root_ui, |ui| {
            draw_ribbon(ui, state, document_bus, storage);
        });

    egui::Panel::bottom("status_bar")
        .exact_size(32.0)
        .show(root_ui, |ui| {
            ui.horizontal_centered(|ui| {
                let slide_count = document_bus
                    .document()
                    .as_ref()
                    .map_or(0, PptxDocument::slide_count);
                let current_slide = if slide_count == 0 {
                    0
                } else {
                    state.selected_slide + 1
                };
                ui.label(format!("Slide {current_slide} of {slide_count}"));
                ui.separator();
                ui.label(&state.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut zoom = state.canvas.zoom_percent();
                    if ui
                        .add(
                            egui::Slider::new(&mut zoom, 10..=400)
                                .suffix("%")
                                .show_value(true),
                        )
                        .changed()
                    {
                        state.canvas.set_zoom_percent(zoom);
                    }
                    if ui
                        .button("Fit")
                        .on_hover_text("Fit slide to canvas")
                        .clicked()
                        && let Some(document) = document_bus.document().as_ref()
                    {
                        state.canvas.fit_to_slide(document.slide_size());
                    }
                });
            });
        });

    egui::Panel::left("slide_thumbnails")
        .default_size(190.0)
        .min_size(150.0)
        .resizable(true)
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Slides");
                ui.add_enabled(false, egui::Button::new("+"))
                    .on_hover_text("Slide insertion is part of a later editing milestone.");
            });
            ui.separator();
            if let Some(document) = document_bus.document().as_ref() {
                for index in 0..document.slide_count() {
                    let selected = state.selected_slide == index;
                    let response = ui.add_sized(
                        [ui.available_width(), 72.0],
                        egui::Button::new(format!("Slide {}", index + 1)).selected(selected),
                    );
                    if response.clicked() {
                        state.selected_slide = index;
                    }
                }
            } else {
                ui.weak("No presentation is open.");
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                if ui.button("Recent files").clicked() {
                    state.backstage = Some(BackstagePage::Recent);
                }
            });
        });

    if let Some(pane) = state.task_pane {
        egui::Panel::right("task_pane")
            .default_size(250.0)
            .min_size(210.0)
            .resizable(true)
            .show(root_ui, |ui| draw_task_pane(ui, state, pane));
    }

    egui::CentralPanel::default().show(root_ui, |ui| {
        if let (Some(document), Some(renderer)) =
            (document_bus.document().as_ref(), canvas_renderer)
        {
            if state.show_notes {
                let canvas_height = (ui.available_height() - 104.0).max(180.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), canvas_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| state.canvas.show(ui, renderer, document.slide_size()),
                );
                ui.separator();
                ui.label("Notes");
                ui.weak("Slide notes remain preserved in the PPTX package; editing arrives with text support.");
            } else {
                state.canvas.show(ui, renderer, document.slide_size());
            }
        } else {
            ui.centered_and_justified(|ui| ui.label("No presentation is open."));
        }
    });
}

fn draw_ribbon(
    ui: &mut egui::Ui,
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    match state.active_tab {
        RibbonTab::Home => {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        unavailable(ui, "Paste", "Clipboard editing is part of M8.");
                        unavailable(ui, "Cut", "Clipboard editing is part of M8.");
                        unavailable(ui, "Copy", "Clipboard editing is part of M8.");
                    });
                    ui.small("Clipboard");
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        if ui.button("New").clicked() {
                            create_blank(state, document_bus);
                        }
                        if ui.button("Open").clicked() {
                            choose_and_open(state, document_bus, storage);
                        }
                        if ui.button("Save").clicked() {
                            save_current(state, document_bus, storage);
                        }
                    });
                    ui.small("Presentation");
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        unavailable(
                            ui,
                            "New Slide",
                            "Slide insertion is part of the shape-editing milestone.",
                        );
                        unavailable(
                            ui,
                            "Layout",
                            "Slide layouts are edited with themes and masters.",
                        );
                        unavailable(ui, "Reset", "No editable slide objects are selected.");
                    });
                    ui.small("Slides");
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        unavailable(ui, "Font", "Text formatting is part of M5.");
                        unavailable(ui, "Paragraph", "Text formatting is part of M5.");
                        unavailable(ui, "Arrange", "Shape editing is part of M4.");
                    });
                    ui.small("Text and drawing");
                });
            });
        }
        RibbonTab::Insert => draw_insert_ribbon(ui),
        RibbonTab::Design => draw_design_ribbon(ui),
        RibbonTab::Transitions => draw_transitions_ribbon(ui),
        RibbonTab::Animations => draw_animations_ribbon(ui),
        RibbonTab::SlideShow => draw_slideshow_ribbon(ui),
        RibbonTab::Review => draw_review_ribbon(ui),
        RibbonTab::View => {
            let slide_size = document_bus
                .document()
                .as_ref()
                .map_or(SlideSize::WIDESCREEN, PptxDocument::slide_size);
            draw_view_ribbon(ui, state, slide_size);
        }
    }
}

fn draw_insert_ribbon(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for label in ["Table", "Picture", "Shape", "Text Box"] {
            unavailable(
                ui,
                label,
                "Slide objects are added in the shape and content milestones.",
            );
        }
        ui.separator();
        for label in ["Chart", "SmartArt", "Video", "Audio"] {
            unavailable(
                ui,
                label,
                "This content type is not available in the current milestone.",
            );
        }
        ui.separator();
        unavailable(
            ui,
            "Link",
            "Hyperlink editing is not available in the current milestone.",
        );
        unavailable(ui, "Comment", "Comments are part of M12.");
    });
}

fn draw_design_ribbon(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        unavailable(ui, "Themes", "Theme editing is part of M7.");
        unavailable(ui, "Variants", "Theme editing is part of M7.");
        ui.separator();
        unavailable(ui, "Format Background", "Slide backgrounds are part of M7.");
        unavailable(
            ui,
            "Slide Size",
            "Changing slide size requires structured OOXML editing.",
        );
    });
}

fn draw_transitions_ribbon(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for label in [
            "None",
            "Fade",
            "Push",
            "Wipe",
            "Split",
            "Reveal",
            "Random Bars",
            "Shape",
        ] {
            unavailable(ui, label, "Transition editing is part of M9.");
        }
        ui.separator();
        unavailable(ui, "Effect Options", "Transition editing is part of M9.");
        unavailable(ui, "Apply to All", "Transition editing is part of M9.");
    });
}

fn draw_animations_ribbon(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for label in [
            "Add Animation",
            "Effect Options",
            "Animation Pane",
            "Trigger",
            "Timing",
            "Reorder",
        ] {
            unavailable(ui, label, "Animation editing is part of M9.");
        }
    });
}

fn draw_slideshow_ribbon(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for label in [
            "From Beginning",
            "From Current Slide",
            "Presenter View",
            "Set Up Slide Show",
            "Rehearse Timings",
        ] {
            unavailable(ui, label, "Slideshow support is part of M10.");
        }
    });
}

fn draw_review_ribbon(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for label in [
            "Spelling",
            "New Comment",
            "Delete Comment",
            "Accessibility",
            "Reading Order",
        ] {
            unavailable(ui, label, "Review tools are added in later milestones.");
        }
    });
}

fn draw_view_ribbon(ui: &mut egui::Ui, state: &mut UiState, slide_size: SlideSize) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            let mut rulers = state.canvas.rulers_visible();
            if ui.checkbox(&mut rulers, "Ruler").changed() {
                state.canvas.set_rulers_visible(rulers);
            }
            let mut grid = state.canvas.grid_visible();
            if ui.checkbox(&mut grid, "Gridlines").changed() {
                state.canvas.set_grid_visible(grid);
            }
            let mut snap = state.canvas.snap_to_grid();
            if ui.checkbox(&mut snap, "Snap to Grid").changed() {
                state.canvas.set_snap_to_grid(snap);
            }
            ui.small("Show");
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.checkbox(&mut state.show_notes, "Notes Pane");
            let mut show_pane = state.task_pane.is_some();
            if ui.checkbox(&mut show_pane, "Selection Pane").changed() {
                state.task_pane = show_pane.then_some(TaskPane::Selection);
            }
            ui.small("Panes");
        });
        ui.separator();
        ui.vertical(|ui| {
            for (pane, label) in [
                (TaskPane::FormatShape, "Format Shape"),
                (TaskPane::FormatPicture, "Format Picture"),
                (TaskPane::Selection, "Selection"),
                (TaskPane::AltText, "Alt Text"),
            ] {
                if ui
                    .selectable_label(state.task_pane == Some(pane), label)
                    .clicked()
                {
                    state.task_pane = Some(pane);
                }
            }
            ui.small("Task panes");
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui.button("Fit Slide").clicked() {
                    state.canvas.fit_to_slide(slide_size);
                }
                if ui.button("Clear Guides").clicked() {
                    state.canvas.clear_guides();
                }
            });
            ui.small(format!("{} guide(s)", state.canvas.guide_count()));
        });
        ui.separator();
        unavailable(
            ui,
            "Slide Sorter",
            "Additional presentation views are part of a later milestone.",
        );
        unavailable(ui, "Reading View", "Slideshow support is part of M10.");
    });
}

fn unavailable(ui: &mut egui::Ui, label: &str, reason: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_hover_text(reason);
}

fn draw_task_pane(ui: &mut egui::Ui, state: &mut UiState, pane: TaskPane) {
    ui.horizontal(|ui| {
        ui.heading(pane.label());
        if ui.small_button("×").clicked() {
            state.task_pane = None;
        }
    });
    ui.separator();
    match pane {
        TaskPane::FormatShape => {
            ui.label("Select a shape to view its fill, line, and effect settings.");
        }
        TaskPane::FormatPicture => {
            ui.label("Select a picture to view its crop and correction settings.");
        }
        TaskPane::Selection => {
            ui.label("Selection pane");
            ui.weak("Object selection becomes available with slide-content editing.");
        }
        TaskPane::AltText => {
            ui.label("Alternative text");
            ui.weak("Choose an object to edit its accessibility description.");
        }
    };
}

fn draw_backstage(
    root_ui: &mut egui::Ui,
    context: &Context,
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    let page = state.backstage.unwrap_or(BackstagePage::Info);
    root_ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(205.0);
            for (candidate, label) in [
                (BackstagePage::Info, "Info"),
                (BackstagePage::New, "New"),
                (BackstagePage::Open, "Open"),
                (BackstagePage::Recent, "Recent"),
                (BackstagePage::Save, "Save"),
                (BackstagePage::SaveAs, "Save As"),
                (BackstagePage::SaveCopy, "Save a Copy"),
                (BackstagePage::Print, "Print"),
                (BackstagePage::Export, "Export"),
                (BackstagePage::Reveal, "Reveal in File Manager"),
                (BackstagePage::Options, "Options"),
                (BackstagePage::About, "About"),
            ] {
                if ui.selectable_label(page == candidate, label).clicked() {
                    state.backstage = Some(candidate);
                    if candidate == BackstagePage::Open {
                        choose_and_open(state, document_bus, storage);
                    } else if candidate == BackstagePage::Save {
                        save_current(state, document_bus, storage);
                    } else if candidate == BackstagePage::SaveAs {
                        choose_save_path(state, document_bus, storage);
                    } else if candidate == BackstagePage::SaveCopy {
                        choose_save_copy(state, document_bus, storage);
                    } else if candidate == BackstagePage::Reveal {
                        reveal_current_file(state);
                    }
                }
            }
        });
        ui.separator();
        ui.vertical(|ui| match page {
            BackstagePage::Info => draw_file_info(ui, state, document_bus),
            BackstagePage::New => {
                ui.heading("New presentation");
                ui.label("Start with a blank OOXML presentation.");
                if ui.button("Blank widescreen").clicked() {
                    create_blank_with_size(state, document_bus, SlideSize::WIDESCREEN);
                    state.backstage = None;
                }
                if ui.button("Blank 4:3").clicked() {
                    create_blank_with_size(state, document_bus, SlideSize::CLASSIC_4_3);
                    state.backstage = None;
                }
            }
            BackstagePage::Open => {
                ui.heading("Open");
                if ui.button("Browse for a presentation…").clicked() {
                    choose_and_open(state, document_bus, storage);
                }
            }
            BackstagePage::Recent => draw_recent_files(ui, state, document_bus, storage),
            BackstagePage::Save => {
                ui.heading("Save");
                if ui.button("Save this presentation").clicked() {
                    save_current(state, document_bus, storage);
                }
            }
            BackstagePage::SaveAs => {
                ui.heading("Save As");
                if ui.button("Choose a PPTX destination…").clicked() {
                    choose_save_path(state, document_bus, storage);
                }
            }
            BackstagePage::SaveCopy => {
                ui.heading("Save a Copy");
                if ui.button("Choose a copy destination…").clicked() {
                    choose_save_copy(state, document_bus, storage);
                }
            }
            BackstagePage::Print => {
                ui.heading("Print");
                ui.label("Print preview and printer output are part of the export milestone.");
            }
            BackstagePage::Export => {
                ui.heading("Export");
                ui.label("PDF and image export are part of the export milestone.");
            }
            BackstagePage::Reveal => {
                ui.heading("Reveal in File Manager");
                if ui.button("Show current file").clicked() {
                    reveal_current_file(state);
                }
            }
            BackstagePage::Options => draw_options(ui, context, state, storage),
            BackstagePage::About => {
                ui.heading("About Presentationpoint");
                ui.label("A local, offline PowerPoint-compatible editor.");
                ui.label(format!(
                    "Version {} · Rust desktop application",
                    env!("CARGO_PKG_VERSION")
                ));
                ui.label("Files remain on this device; the application makes no network requests.");
            }
        });
    });
}

fn draw_file_info(
    ui: &mut egui::Ui,
    state: &UiState,
    document_bus: &CommandBus<Option<PptxDocument>>,
) {
    ui.heading("Presentation information");
    if let Some(document) = document_bus.document().as_ref() {
        let size = document.slide_size();
        ui.label(format!("Slides: {}", document.slide_count()));
        ui.label(format!(
            "Slide size: {:.2} × {:.2} inches",
            size.width.0 as f64 / 914_400.0,
            size.height.0 as f64 / 914_400.0
        ));
        ui.label(format!("Package parts: {}", document.part_names().count()));
        ui.label(format!(
            "Location: {}",
            state.current_path.as_ref().map_or_else(
                || "New presentation".to_owned(),
                |path| path.display().to_string()
            )
        ));
    } else {
        ui.label("No presentation is open.");
    }
}

fn draw_recent_files(
    ui: &mut egui::Ui,
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    ui.heading("Recent presentations");
    if state.recent_paths.is_empty() {
        ui.weak("No recent files");
        return;
    }
    let mut selected = None;
    for path in &state.recent_paths {
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Presentation");
        if ui.button(format!("{label}\n{}", path.display())).clicked() {
            selected = Some(path.clone());
        }
    }
    if let Some(path) = selected {
        open_path(&path, state, document_bus, storage);
        if state.current_path.as_deref() == Some(path.as_path()) {
            state.backstage = None;
        }
    }
}

fn draw_options(ui: &mut egui::Ui, context: &Context, state: &mut UiState, storage: &AppStorage) {
    ui.heading("Options");
    let mut scale = state.settings.ui_scale_percent;
    if ui
        .add(egui::Slider::new(&mut scale, 80..=200).suffix("% UI scale"))
        .changed()
    {
        state.settings.ui_scale_percent = scale;
        context.set_zoom_factor(f32::from(scale) / 100.0);
        state.status = match storage.save_settings(&state.settings) {
            Ok(()) => "Interface scale saved locally".to_owned(),
            Err(error) => format!("Interface scale changed but could not be saved: {error}"),
        };
    }
    ui.label("Settings are stored in the local application data folder.");
}

fn reveal_current_file(state: &mut UiState) {
    let Some(path) = state.current_path.clone() else {
        state.status = "Save the presentation before revealing its file location".to_owned();
        return;
    };
    #[cfg(windows)]
    let result = ProcessCommand::new("explorer")
        .arg("/select,")
        .arg(&path)
        .spawn();
    #[cfg(not(windows))]
    let result: Result<std::process::Child, std::io::Error> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "file manager reveal is only available on Windows",
    ));
    state.status = match result {
        Ok(_) => format!("Revealed {} in File Explorer", path.display()),
        Err(error) => format!("Could not reveal file: {error}"),
    };
}

fn create_blank(state: &mut UiState, document_bus: &mut CommandBus<Option<PptxDocument>>) {
    create_blank_with_size(state, document_bus, SlideSize::WIDESCREEN);
}

fn create_blank_with_size(
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    slide_size: SlideSize,
) {
    match PptxDocument::new_minimal(slide_size) {
        Ok(document) => match document_bus.execute(ReplaceState::new(Some(document))) {
            Ok(()) => {
                document_bus.clear_history();
                state.current_path = None;
                state.selected_slide = 0;
                state.canvas.clear_guides();
                state.status = if slide_size == SlideSize::WIDESCREEN {
                    "Blank widescreen presentation created".to_owned()
                } else {
                    "Blank 4:3 presentation created".to_owned()
                };
            }
            Err(error) => state.status = error.to_string(),
        },
        Err(error) => state.status = error.to_string(),
    }
}

fn choose_and_open(
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    let selected = FileDialog::new()
        .add_filter("PowerPoint presentation", &["pptx"])
        .pick_file();
    if let Some(path) = selected {
        open_path(&path, state, document_bus, storage);
    }
}

fn open_path(
    path: &Path,
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    match PptxDocument::open(path) {
        Ok(document) => match document_bus.execute(ReplaceState::new(Some(document))) {
            Ok(()) => {
                document_bus.clear_history();
                match storage.add_recent(path) {
                    Ok(recent_paths) => {
                        state.recent_paths = recent_paths;
                        state.current_path = Some(path.to_path_buf());
                        state.status = format!("Opened {}", path.display());
                    }
                    Err(error) => {
                        state.current_path = Some(path.to_path_buf());
                        state.status = format!("Opened deck; recent-file update failed: {error}");
                    }
                }
                state.selected_slide = 0;
                state.canvas.clear_guides();
                state.backstage = None;
            }
            Err(error) => state.status = error.to_string(),
        },
        Err(error) => state.status = format!("Could not open {}: {error}", path.display()),
    }
}

fn save_current(
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    if let Some(path) = state.current_path.clone() {
        save_to_path(&path, state, document_bus, storage);
    } else {
        choose_save_path(state, document_bus, storage);
    }
}

fn choose_save_path(
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    let selected = FileDialog::new()
        .add_filter("PowerPoint presentation", &["pptx"])
        .set_file_name("Presentation.pptx")
        .save_file();
    if let Some(path) = selected {
        save_to_path(&path, state, document_bus, storage);
    }
}

fn choose_save_copy(
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    let Some(current_path) = state.current_path.as_ref() else {
        choose_save_path(state, document_bus, storage);
        return;
    };
    let file_name = current_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Presentation.pptx");
    let selected = FileDialog::new()
        .add_filter("PowerPoint presentation", &["pptx"])
        .set_file_name(file_name)
        .save_file();
    if let Some(path) = selected {
        save_copy_to_path(&path, state, document_bus, storage);
    }
}

fn save_copy_to_path(
    path: &Path,
    state: &mut UiState,
    document_bus: &CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    let Some(document) = document_bus.document().as_ref() else {
        state.status = "There is no presentation to save".to_owned();
        return;
    };
    match document.save_atomic(path) {
        Ok(()) => {
            state.status = match storage.add_recent(path) {
                Ok(recent_paths) => {
                    state.recent_paths = recent_paths;
                    format!("Saved a copy to {}", path.display())
                }
                Err(error) => format!("Saved a copy; recent-file update failed: {error}"),
            };
        }
        Err(error) => state.status = format!("Could not save copy {}: {error}", path.display()),
    }
}

fn save_to_path(
    path: &Path,
    state: &mut UiState,
    document_bus: &mut CommandBus<Option<PptxDocument>>,
    storage: &AppStorage,
) {
    let Some(document) = document_bus.document().as_ref() else {
        state.status = "There is no presentation to save".to_owned();
        return;
    };
    match document.save_atomic(path) {
        Ok(()) => {
            state.current_path = Some(path.to_path_buf());
            document_bus.clear_history();
            state.status = match storage.add_recent(path) {
                Ok(recent_paths) => {
                    state.recent_paths = recent_paths;
                    format!("Saved {}", path.display())
                }
                Err(error) => format!("Saved deck; recent-file update failed: {error}"),
            };
        }
        Err(error) => state.status = format!("Could not save {}: {error}", path.display()),
    }
}

pub fn run() -> Result<(), AppError> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [] => run_desktop(),
        [option, output] if option == "--generate-samples" => {
            generate_samples(&crate::paths::resolve_project_path(Path::new(output))?)
        }
        [option, input, output] if option == "--roundtrip-corpus" => roundtrip_corpus(
            &crate::paths::resolve_project_path(Path::new(input))?,
            &crate::paths::resolve_project_path(Path::new(output))?,
        ),
        _ => Err(AppError::CommandLine(
            "usage: presentationpoint [--generate-samples DIR | --roundtrip-corpus INPUT_DIR OUTPUT_DIR]".to_owned(),
        )),
    }
}

fn run_desktop() -> Result<(), AppError> {
    let storage = AppStorage::local()?;
    let mut application = PresentationpointApp::new(storage)?;
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut application)?;
    if let Some(error) = application.startup_error {
        return Err(error);
    }
    Ok(())
}

fn generate_samples(directory: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(directory).map_err(|source| AppError::io(directory, source))?;
    let samples = [
        ("blank-widescreen.pptx", SlideSize::WIDESCREEN),
        ("blank-classic-4-3.pptx", SlideSize::CLASSIC_4_3),
    ];
    for (name, size) in samples {
        let path = directory.join(name);
        PptxDocument::new_minimal(size)?.save_atomic(&path)?;
        tracing::info!(path = %path.display(), "Generated an OOXML sample presentation");
    }
    Ok(())
}

fn roundtrip_corpus(input: &Path, output: &Path) -> Result<(), AppError> {
    let entries = std::fs::read_dir(input).map_err(|source| AppError::io(input, source))?;
    std::fs::create_dir_all(output).map_err(|source| AppError::io(output, source))?;
    let mut presentations = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| AppError::io(input, source))?;
        let path = entry.path();
        let is_pptx = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pptx"));
        if path.is_file() && is_pptx {
            presentations.push(path);
        }
    }
    presentations.sort();
    if presentations.is_empty() {
        return Err(AppError::CommandLine(format!(
            "no PPTX files found in {}",
            input.display()
        )));
    }

    for source in &presentations {
        let document = PptxDocument::open(source)?;
        let file_name = source.file_name().ok_or_else(|| {
            AppError::CommandLine(format!("input has no filename: {}", source.display()))
        })?;
        let destination = output.join(file_name);
        document.save_atomic(&destination)?;
        let reopened = PptxDocument::open(&destination)?;
        if !document.matches_package(&reopened) {
            return Err(AppError::InvalidPptx(format!(
                "semantic diff after round-trip: {}",
                source.display()
            )));
        }
        tracing::info!(
            source = %source.display(),
            slides = document.slide_count(),
            "PPTX round-trip passed"
        );
    }
    Ok(())
}
