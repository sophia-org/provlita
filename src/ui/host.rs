//! Supported non-Winit Xilem embedding. This host never initializes a GPU.
use super::{DockState, dock_view};
use crate::config::Config;
use masonry::{
    app::{RenderRoot, RenderRootOptions, WindowSizePolicy},
    core::{DefaultProperties, WidgetId, WidgetRef},
    imaging::record::{Scene, replay_transformed},
    kurbo::Affine,
    peniko::Blob,
};
use std::sync::Arc;
use xilem_masonry::{
    MasonryRoot, ViewCtx,
    core::{
        DynMessage, MessageCtx, MessageResult, ProxyError, RawProxy, SendMessage, View, ViewId,
        ViewPathTracker,
    },
};

#[derive(Debug)]
struct NoAsync;
impl RawProxy for NoAsync {
    fn send_message(&self, _: Arc<[ViewId]>, message: SendMessage) -> Result<(), ProxyError> {
        Err(ProxyError::DriverFinished(message))
    }
    fn dyn_debug(&self) -> &dyn std::fmt::Debug {
        self
    }
}

/// Masonry paint output; neither GPU completion nor native presentation.
pub struct DockScene {
    /// Recorded commands from the retained widget tree.
    pub scene: Scene,
    /// Physical width.
    pub width: u32,
    /// Physical height.
    pub height: u32,
}

/// One retained tree. The service must key its lifetime by exact admitted
/// grant/output/allocation/scale; this host itself grants none of those rights.
pub struct DockHost {
    state: DockState,
    view: MasonryRoot<DockState>,
    view_state: <MasonryRoot<DockState> as View<DockState, (), ViewCtx>>::ViewState,
    context: ViewCtx,
    root: RenderRoot,
    width: u32,
    height: u32,
    scale: f64,
}
impl DockHost {
    /// Construct a bounded retained host without display or system font access.
    pub fn new(config: &Config, scale: u32) -> Result<Self, String> {
        if !(1..=4).contains(&scale) || config.pins.is_empty() || config.pins.len() > 16 {
            return Err("invalid dock extent or scale".into());
        }
        let (width, height) = config.logical_size();
        let width = width.checked_mul(scale).ok_or("dock width overflow")?;
        let height = height.checked_mul(scale).ok_or("dock height overflow")?;
        if width > 8192 || height > 1024 || u64::from(width) * u64::from(height) > 8 * 1024 * 1024 {
            return Err("dock raster exceeds local budget".into());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|e| e.to_string())?;
        let mut context = ViewCtx::new(Arc::new(NoAsync), Arc::new(runtime));
        let mut state = DockState::new(config);
        let view = MasonryRoot::new(dock_view(&state));
        let (element, view_state) = view.build(&mut context, &mut state);
        let root = RenderRoot::new(
            element.0.new_widget,
            |_| {},
            RenderRootOptions {
                default_properties: Arc::new(DefaultProperties::new()),
                use_system_fonts: false,
                size_policy: WindowSizePolicy::User,
                size: (width, height).into(),
                scale_factor: f64::from(scale),
                test_font: Some(Blob::new(Arc::new(
                    include_bytes!("../../assets/fonts/DejaVuSansMono.ttf").to_vec(),
                ))),
            },
        );
        Ok(Self {
            state,
            view,
            view_state,
            context,
            root,
            width,
            height,
            scale: f64::from(scale),
        })
    }
    /// Current Xilem application state, without mutable toolkit access.
    pub fn state(&self) -> &DockState {
        &self.state
    }

    /// Apply a complete catalog observation and reconcile the existing view.
    pub fn install_catalog(
        &mut self,
        generation: u64,
        entries: &[(String, u16, bool)],
    ) -> Result<(), &'static str> {
        self.state.install_catalog(generation, entries)?;
        self.rebuild();
        Ok(())
    }
    fn rebuild(&mut self) {
        let next = MasonryRoot::new(dock_view(&self.state));
        next.rebuild(
            &self.view,
            &mut self.view_state,
            &mut self.context,
            &mut self.root,
            &mut self.state,
        );
        self.view = next;
    }
    /// Deliver a Masonry action through Xilem's actual message path. A caller
    /// must not substitute this for the protocol's exact presented input checks.
    pub fn dispatch_widget_action(&mut self, id: WidgetId, action: DynMessage) -> bool {
        let Some(path) = self.context.get_id_path(id).cloned() else {
            return false;
        };
        let mut message = MessageCtx::new(std::mem::take(self.context.environment()), path, action);
        let result = self.view.message(
            &mut self.view_state,
            &mut message,
            &mut self.root,
            &mut self.state,
        );
        let (environment, _, _) = message.finish();
        *self.context.environment() = environment;
        match result {
            MessageResult::Action(()) | MessageResult::RequestRebuild => {
                self.rebuild();
                true
            }
            MessageResult::Nop => true,
            MessageResult::Stale => false,
        }
    }
    /// Transfer the bounded callback request to the protocol effect adapter.
    pub fn take_intent(&mut self) -> Option<super::TileIntent> {
        self.state.take_intent()
    }
    /// Produce a scene from this tree; no rasterizer or GPU is invoked here.
    pub fn scene(&mut self) -> DockScene {
        let (layers, _) = self.root.redraw();
        let mut logical = Scene::new();
        layers.replay_into(&mut logical);
        let mut scene = Scene::new();
        replay_transformed(&logical, &mut scene, Affine::scale(self.scale));
        DockScene {
            scene,
            width: self.width,
            height: self.height,
        }
    }
    /// Tile widget identities and layout bounds from the same retained tree.
    /// Native target identities must be assigned separately by the content owner.
    pub fn tile_layout(&self) -> Vec<(WidgetId, masonry::kurbo::Rect)> {
        fn collect(
            widget: WidgetRef<'_, dyn masonry::core::Widget>,
            result: &mut Vec<(WidgetId, masonry::kurbo::Rect)>,
        ) {
            if widget.downcast::<masonry::widgets::Button>().is_some() {
                // Paint bounds include descendants and are not hit rectangles.
                let context = widget.ctx();
                result.push((
                    widget.id(),
                    context
                        .window_transform()
                        .transform_rect_bbox(context.border_box()),
                ));
            }
            for child in widget.children() {
                collect(child, result);
            }
        }
        let mut result = Vec::new();
        collect(self.root.get_layer_root(0), &mut result);
        result
    }
}
impl Drop for DockHost {
    fn drop(&mut self) {
        self.view
            .teardown(&mut self.view_state, &mut self.context, &mut self.root);
    }
}
