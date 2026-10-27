//! Handling of Wayland color management (`wp_color_manager_v1`).

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

use sctk::globals::GlobalData;
use sctk::reexports::client::globals::{BindError, GlobalList};
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{
    delegate_dispatch, Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use sctk::reexports::protocols::wp::color_management::v1::client::wp_color_management_surface_feedback_v1::{
    Event as FeedbackEvent, WpColorManagementSurfaceFeedbackV1,
};
use sctk::reexports::protocols::wp::color_management::v1::client::wp_color_management_surface_v1::WpColorManagementSurfaceV1;
use sctk::reexports::protocols::wp::color_management::v1::client::wp_color_manager_v1::{
    Event as ColorManagerEvent, Feature, Primaries, RenderIntent, TransferFunction,
    WpColorManagerV1,
};
use sctk::reexports::protocols::wp::color_management::v1::client::wp_image_description_creator_params_v1::WpImageDescriptionCreatorParamsV1;
use sctk::reexports::protocols::wp::color_management::v1::client::wp_image_description_info_v1::{
    Event as ImageDescriptionInfoEvent, WpImageDescriptionInfoV1,
};
use sctk::reexports::protocols::wp::color_management::v1::client::wp_image_description_v1::{
    Event as ImageDescriptionEvent, WpImageDescriptionV1,
};

use crate::platform_impl::wayland::state::WinitState;

/// Color space encoding requested for a Wayland surface via `wp_color_manager_v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum WaylandSurfaceColorSpace {
    /// Automatically select the surface color space based on the compositor's
    /// preferred image description (`wp_color_management_surface_feedback_v1`).
    #[default]
    Auto = 0,
    /// Standard sRGB (IEC 61966-2-1 / BT.709 primaries).
    Srgb = 1,
    /// Display P3 (SMPTE EG 432-1 primaries, sRGB transfer function).
    DisplayP3 = 2,
    /// Adobe RGB (1998) (ISO 12640-4 primaries, Gamma 2.2 transfer function).
    AdobeRgb = 3,
}

impl WaylandSurfaceColorSpace {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Srgb,
            2 => Self::DisplayP3,
            3 => Self::AdobeRgb,
            _ => Self::Auto,
        }
    }
}

/// Detected preferred color space of the active monitor reported by the Wayland compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum WaylandDetectedGamut {
    /// Unknown or `wp_color_manager_v1` not advertised by the compositor.
    #[default]
    Unknown = 0,
    /// Standard sRGB gamut monitor.
    Srgb = 1,
    /// Wide-gamut monitor with Display P3 / DCI-P3 / BT.2020 or custom ICC profile.
    DisplayP3 = 2,
    /// Wide-gamut monitor with Adobe RGB (1998) primaries.
    AdobeRgb = 3,
}

impl WaylandDetectedGamut {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Srgb,
            2 => Self::DisplayP3,
            3 => Self::AdobeRgb,
            _ => Self::Unknown,
        }
    }
}

/// Snapshot of the Wayland `wp_color_manager_v1` state for UI inspection and rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WaylandColorManagementStatus {
    /// Whether `wp_color_manager_v1` is supported and bound on the active Wayland connection.
    pub protocol_supported: bool,
    /// Whether parametric image descriptions (`DisplayP3` / `AdobeRgb`) are supported.
    pub parametric_supported: bool,
    /// Detected preferred color gamut of the monitor.
    pub monitor_detected_gamut: WaylandDetectedGamut,
    /// Whether an explicit ICC profile fd was reported by the compositor for the monitor.
    pub monitor_has_icc_profile: bool,
    /// Currently active concrete surface color space (`Srgb`, `DisplayP3`, or `AdobeRgb`).
    pub active_surface_color_space: WaylandSurfaceColorSpace,
}

static PROTOCOL_SUPPORTED: AtomicBool = AtomicBool::new(false);
static PARAMETRIC_SUPPORTED: AtomicBool = AtomicBool::new(false);
static MONITOR_DETECTED_GAMUT: AtomicU8 = AtomicU8::new(WaylandDetectedGamut::Unknown as u8);
static MONITOR_HAS_ICC: AtomicBool = AtomicBool::new(false);
static REQUESTED_SURFACE_COLOR_SPACE: AtomicU8 =
    AtomicU8::new(WaylandSurfaceColorSpace::Auto as u8);
static ACTIVE_SURFACE_COLOR_SPACE: AtomicU8 = AtomicU8::new(WaylandSurfaceColorSpace::Srgb as u8);

/// Query the current Wayland color management status.
pub fn wayland_color_management_status() -> WaylandColorManagementStatus {
    WaylandColorManagementStatus {
        protocol_supported: PROTOCOL_SUPPORTED.load(Ordering::Relaxed),
        parametric_supported: PARAMETRIC_SUPPORTED.load(Ordering::Relaxed),
        monitor_detected_gamut: WaylandDetectedGamut::from_u8(
            MONITOR_DETECTED_GAMUT.load(Ordering::Relaxed),
        ),
        monitor_has_icc_profile: MONITOR_HAS_ICC.load(Ordering::Relaxed),
        active_surface_color_space: WaylandSurfaceColorSpace::from_u8(
            ACTIVE_SURFACE_COLOR_SPACE.load(Ordering::Relaxed),
        ),
    }
}

/// Set the desired Wayland surface color space mode (`Auto`, `Srgb`, `DisplayP3`, `AdobeRgb`).
pub fn set_wayland_surface_color_space(mode: WaylandSurfaceColorSpace) {
    REQUESTED_SURFACE_COLOR_SPACE.store(mode as u8, Ordering::Relaxed);
}

/// Resolve the concrete surface color space (`Srgb`, `DisplayP3`, or `AdobeRgb`) for a given mode.
pub fn resolve_surface_color_space(mode: WaylandSurfaceColorSpace) -> WaylandSurfaceColorSpace {
    if !PROTOCOL_SUPPORTED.load(Ordering::Relaxed) || !PARAMETRIC_SUPPORTED.load(Ordering::Relaxed)
    {
        return WaylandSurfaceColorSpace::Srgb;
    }

    match mode {
        WaylandSurfaceColorSpace::Auto => {
            match WaylandDetectedGamut::from_u8(MONITOR_DETECTED_GAMUT.load(Ordering::Relaxed)) {
                WaylandDetectedGamut::AdobeRgb => WaylandSurfaceColorSpace::AdobeRgb,
                WaylandDetectedGamut::DisplayP3 => WaylandSurfaceColorSpace::DisplayP3,
                WaylandDetectedGamut::Srgb | WaylandDetectedGamut::Unknown => {
                    WaylandSurfaceColorSpace::Srgb
                },
            }
        },
        concrete => concrete,
    }
}

/// Wayland `wp_color_manager_v1` state.
#[derive(Debug)]
pub struct ColorManagementManager {
    manager: WpColorManagerV1,
    pub features: Vec<Feature>,
    pub intents: Vec<RenderIntent>,
    pub tfs: Vec<TransferFunction>,
    pub primaries: Vec<Primaries>,
    pub done: bool,
}

impl ColorManagementManager {
    pub fn new(
        globals: &GlobalList,
        queue_handle: &QueueHandle<WinitState>,
    ) -> Result<Self, BindError> {
        let manager = globals.bind(queue_handle, 1..=2, GlobalData)?;
        PROTOCOL_SUPPORTED.store(true, Ordering::Relaxed);
        Ok(Self {
            manager,
            features: Vec::new(),
            intents: Vec::new(),
            tfs: Vec::new(),
            primaries: Vec::new(),
            done: false,
        })
    }

    pub fn create_surface(
        &self,
        surface: &WlSurface,
        queue_handle: &QueueHandle<WinitState>,
    ) -> WindowColorManagement {
        let cm_surface = self.manager.get_surface(surface, queue_handle, GlobalData);
        let feedback = self.manager.get_surface_feedback(
            surface,
            queue_handle,
            SurfaceFeedbackData { surface: surface.clone() },
        );
        // Request initial preferred image description for this window surface.
        let _preferred = feedback.get_preferred(
            queue_handle,
            ImageDescriptionData::Feedback { surface: surface.clone() },
        );

        WindowColorManagement {
            cm_surface,
            feedback,
            applied_space: WaylandSurfaceColorSpace::Srgb,
            pending_space: None,
        }
    }

    pub fn preferred_render_intent(&self) -> RenderIntent {
        if self.intents.contains(&RenderIntent::RelativeBpc) {
            RenderIntent::RelativeBpc
        } else if self.intents.contains(&RenderIntent::Relative) {
            RenderIntent::Relative
        } else {
            RenderIntent::Perceptual
        }
    }

    pub fn sync_window_color_space(
        &self,
        surface: &WlSurface,
        win_cm: &mut WindowColorManagement,
        queue_handle: &QueueHandle<WinitState>,
    ) {
        if !self.done {
            return;
        }

        let requested = WaylandSurfaceColorSpace::from_u8(
            REQUESTED_SURFACE_COLOR_SPACE.load(Ordering::Relaxed),
        );
        let target = resolve_surface_color_space(requested);

        if win_cm.applied_space == target || win_cm.pending_space == Some(target) {
            return;
        }

        if target == WaylandSurfaceColorSpace::Srgb {
            win_cm.cm_surface.unset_image_description();
            win_cm.applied_space = WaylandSurfaceColorSpace::Srgb;
            win_cm.pending_space = None;
            ACTIVE_SURFACE_COLOR_SPACE
                .store(WaylandSurfaceColorSpace::Srgb as u8, Ordering::Relaxed);
            surface.commit();
            return;
        }

        if !self.features.contains(&Feature::Parametric) {
            return;
        }

        let creator = self.manager.create_parametric_creator(queue_handle, GlobalData);

        let primaries_ok = match target {
            WaylandSurfaceColorSpace::DisplayP3 => {
                if self.primaries.contains(&Primaries::DisplayP3) {
                    creator.set_primaries_named(Primaries::DisplayP3);
                    true
                } else if self.features.contains(&Feature::SetPrimaries) {
                    creator.set_primaries(
                        680_000, 320_000, 265_000, 690_000, 150_000, 60_000, 312_700, 329_000,
                    );
                    true
                } else {
                    false
                }
            },
            WaylandSurfaceColorSpace::AdobeRgb => {
                if self.primaries.contains(&Primaries::AdobeRgb) {
                    creator.set_primaries_named(Primaries::AdobeRgb);
                    true
                } else if self.features.contains(&Feature::SetPrimaries) {
                    creator.set_primaries(
                        640_000, 330_000, 210_000, 710_000, 150_000, 60_000, 312_700, 329_000,
                    );
                    true
                } else {
                    false
                }
            },
            WaylandSurfaceColorSpace::Srgb | WaylandSurfaceColorSpace::Auto => false,
        };

        let tf_ok = match target {
            WaylandSurfaceColorSpace::AdobeRgb => {
                if self.tfs.contains(&TransferFunction::Gamma22) {
                    creator.set_tf_named(TransferFunction::Gamma22);
                    true
                } else if self.features.contains(&Feature::SetTfPower) {
                    creator.set_tf_power(21_992);
                    true
                } else if self.tfs.contains(&TransferFunction::CompoundPower24) {
                    creator.set_tf_named(TransferFunction::CompoundPower24);
                    true
                } else if self.tfs.contains(&TransferFunction::Srgb) {
                    creator.set_tf_named(TransferFunction::Srgb);
                    true
                } else {
                    false
                }
            },
            WaylandSurfaceColorSpace::DisplayP3 => {
                if self.tfs.contains(&TransferFunction::CompoundPower24) {
                    creator.set_tf_named(TransferFunction::CompoundPower24);
                    true
                } else if self.tfs.contains(&TransferFunction::Srgb) {
                    creator.set_tf_named(TransferFunction::Srgb);
                    true
                } else if self.tfs.contains(&TransferFunction::Gamma22) {
                    creator.set_tf_named(TransferFunction::Gamma22);
                    true
                } else {
                    false
                }
            },
            WaylandSurfaceColorSpace::Srgb | WaylandSurfaceColorSpace::Auto => false,
        };

        if primaries_ok && tf_ok {
            win_cm.pending_space = Some(target);
            let _desc = creator.create(
                queue_handle,
                ImageDescriptionData::ApplyToSurface { surface: surface.clone(), target },
            );
        }
    }
}

/// Per-window color management protocol objects.
#[derive(Debug)]
pub struct WindowColorManagement {
    pub cm_surface: WpColorManagementSurfaceV1,
    pub feedback: WpColorManagementSurfaceFeedbackV1,
    pub applied_space: WaylandSurfaceColorSpace,
    pub pending_space: Option<WaylandSurfaceColorSpace>,
}

impl WindowColorManagement {
    pub fn destroy(self) {
        self.feedback.destroy();
        self.cm_surface.destroy();
    }
}

#[derive(Debug, Clone)]
pub struct SurfaceFeedbackData {
    pub surface: WlSurface,
}

#[derive(Debug, Clone)]
pub enum ImageDescriptionData {
    Feedback { surface: WlSurface },
    ApplyToSurface { surface: WlSurface, target: WaylandSurfaceColorSpace },
}

#[derive(Debug, Default)]
pub struct PreferredInfoAccumulator {
    pub primaries_named: Option<Primaries>,
    pub primaries_xy: Option<(i32, i32, i32, i32, i32, i32)>,
    pub target_primaries_xy: Option<(i32, i32, i32, i32, i32, i32)>,
    pub has_icc_file: bool,
}

impl PreferredInfoAccumulator {
    fn classify_gamut(&self) -> WaylandDetectedGamut {
        if let Some(named) = self.primaries_named {
            match named {
                Primaries::AdobeRgb => return WaylandDetectedGamut::AdobeRgb,
                Primaries::DisplayP3 | Primaries::DciP3 | Primaries::Bt2020 => {
                    return WaylandDetectedGamut::DisplayP3;
                },
                Primaries::Srgb => {
                    if self.target_primaries_xy.is_none() && !self.has_icc_file {
                        return WaylandDetectedGamut::Srgb;
                    }
                },
                _ => {},
            }
        }

        let xy = self.primaries_xy.or(self.target_primaries_xy);
        if let Some((r_x, _r_y, g_x, g_y, _b_x, _b_y)) = xy {
            // Adobe RGB (1998) has green (0.21, 0.71) and red (0.64, 0.33).
            if g_x <= 240_000 && g_y >= 680_000 && r_x < 660_000 {
                return WaylandDetectedGamut::AdobeRgb;
            }
            // Display P3 has red (0.68, 0.32) and green (0.265, 0.69); BT.2020 is even wider.
            if r_x >= 655_000 || g_y >= 635_000 || g_x <= 280_000 {
                return WaylandDetectedGamut::DisplayP3;
            }
            return WaylandDetectedGamut::Srgb;
        }

        if self.has_icc_file {
            return WaylandDetectedGamut::DisplayP3;
        }

        WaylandDetectedGamut::Srgb
    }
}

#[derive(Debug, Default)]
pub struct ImageDescriptionInfoData {
    pub surface: Option<WlSurface>,
    pub accum: Mutex<PreferredInfoAccumulator>,
}

impl Dispatch<WpColorManagerV1, GlobalData, WinitState> for ColorManagementManager {
    fn event(
        state: &mut WinitState,
        _proxy: &WpColorManagerV1,
        event: <WpColorManagerV1 as Proxy>::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
        let Some(cm) = state.color_management.as_mut() else {
            return;
        };

        match event {
            ColorManagerEvent::SupportedFeature { feature } => {
                if let WEnum::Value(f) = feature {
                    if f == Feature::Parametric {
                        PARAMETRIC_SUPPORTED.store(true, Ordering::Relaxed);
                    }
                    cm.features.push(f);
                }
            },
            ColorManagerEvent::SupportedIntent { render_intent } => {
                if let WEnum::Value(i) = render_intent {
                    cm.intents.push(i);
                }
            },
            ColorManagerEvent::SupportedTfNamed { tf } => {
                if let WEnum::Value(t) = tf {
                    cm.tfs.push(t);
                }
            },
            ColorManagerEvent::SupportedPrimariesNamed { primaries } => {
                if let WEnum::Value(p) = primaries {
                    cm.primaries.push(p);
                }
            },
            ColorManagerEvent::Done => {
                cm.done = true;
            },
            _ => {},
        }
    }
}

impl Dispatch<WpColorManagementSurfaceV1, GlobalData, WinitState> for ColorManagementManager {
    fn event(
        _state: &mut WinitState,
        _proxy: &WpColorManagementSurfaceV1,
        _event: <WpColorManagementSurfaceV1 as Proxy>::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
    }
}

impl Dispatch<WpColorManagementSurfaceFeedbackV1, SurfaceFeedbackData, WinitState>
    for ColorManagementManager
{
    fn event(
        _state: &mut WinitState,
        proxy: &WpColorManagementSurfaceFeedbackV1,
        event: <WpColorManagementSurfaceFeedbackV1 as Proxy>::Event,
        data: &SurfaceFeedbackData,
        _conn: &Connection,
        qhandle: &QueueHandle<WinitState>,
    ) {
        match event {
            FeedbackEvent::PreferredChanged { .. } | FeedbackEvent::PreferredChanged2 { .. } => {
                let _preferred = proxy.get_preferred(
                    qhandle,
                    ImageDescriptionData::Feedback { surface: data.surface.clone() },
                );
            },
            _ => {},
        }
    }
}

impl Dispatch<WpImageDescriptionCreatorParamsV1, GlobalData, WinitState>
    for ColorManagementManager
{
    fn event(
        _state: &mut WinitState,
        _proxy: &WpImageDescriptionCreatorParamsV1,
        _event: <WpImageDescriptionCreatorParamsV1 as Proxy>::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
    }
}

impl Dispatch<WpImageDescriptionV1, ImageDescriptionData, WinitState> for ColorManagementManager {
    fn event(
        state: &mut WinitState,
        proxy: &WpImageDescriptionV1,
        event: <WpImageDescriptionV1 as Proxy>::Event,
        data: &ImageDescriptionData,
        _conn: &Connection,
        qhandle: &QueueHandle<WinitState>,
    ) {
        match event {
            ImageDescriptionEvent::Ready { .. } | ImageDescriptionEvent::Ready2 { .. } => {
                match data {
                    ImageDescriptionData::Feedback { surface } => {
                        let _info = proxy.get_information(
                            qhandle,
                            ImageDescriptionInfoData {
                                surface: Some(surface.clone()),
                                accum: Mutex::new(PreferredInfoAccumulator::default()),
                            },
                        );
                        proxy.destroy();
                    },
                    ImageDescriptionData::ApplyToSurface { surface, target } => {
                        let intent = state
                            .color_management
                            .as_ref()
                            .map(|cm| cm.preferred_render_intent())
                            .unwrap_or(RenderIntent::Perceptual);
                        let window_id = crate::platform_impl::wayland::make_wid(surface);
                        if let Some(window) = state.windows.get_mut().get(&window_id) {
                            if let Ok(mut win) = window.lock() {
                                if let Some(win_cm) = win.color_management.as_mut() {
                                    win_cm.cm_surface.set_image_description(proxy, intent);
                                    win_cm.applied_space = *target;
                                    win_cm.pending_space = None;
                                    ACTIVE_SURFACE_COLOR_SPACE
                                        .store(*target as u8, Ordering::Relaxed);
                                    surface.commit();
                                }
                            }
                        }
                        proxy.destroy();
                    },
                }
            },
            ImageDescriptionEvent::Failed { .. } => {
                if let ImageDescriptionData::ApplyToSurface { surface, .. } = data {
                    let window_id = crate::platform_impl::wayland::make_wid(surface);
                    if let Some(window) = state.windows.get_mut().get(&window_id) {
                        if let Ok(mut win) = window.lock() {
                            if let Some(win_cm) = win.color_management.as_mut() {
                                win_cm.pending_space = None;
                            }
                        }
                    }
                }
                proxy.destroy();
            },
            _ => {},
        }
    }
}

impl Dispatch<WpImageDescriptionInfoV1, ImageDescriptionInfoData, WinitState>
    for ColorManagementManager
{
    fn event(
        state: &mut WinitState,
        _proxy: &WpImageDescriptionInfoV1,
        event: <WpImageDescriptionInfoInfoEventAlias as Proxy>::Event,
        data: &ImageDescriptionInfoData,
        _conn: &Connection,
        qhandle: &QueueHandle<WinitState>,
    ) {
        match event {
            ImageDescriptionInfoEvent::PrimariesNamed { primaries } => {
                if let WEnum::Value(p) = primaries {
                    if let Ok(mut accum) = data.accum.lock() {
                        accum.primaries_named = Some(p);
                    }
                }
            },
            ImageDescriptionInfoEvent::Primaries { r_x, r_y, g_x, g_y, b_x, b_y, .. } => {
                if let Ok(mut accum) = data.accum.lock() {
                    accum.primaries_xy = Some((r_x, r_y, g_x, g_y, b_x, b_y));
                }
            },
            ImageDescriptionInfoEvent::TargetPrimaries { r_x, r_y, g_x, g_y, b_x, b_y, .. } => {
                if let Ok(mut accum) = data.accum.lock() {
                    accum.target_primaries_xy = Some((r_x, r_y, g_x, g_y, b_x, b_y));
                }
            },
            ImageDescriptionInfoEvent::IccFile { .. } => {
                if let Ok(mut accum) = data.accum.lock() {
                    accum.has_icc_file = true;
                }
            },
            ImageDescriptionInfoEvent::Done => {
                let (detected, has_icc) = if let Ok(accum) = data.accum.lock() {
                    (accum.classify_gamut(), accum.has_icc_file)
                } else {
                    (WaylandDetectedGamut::Srgb, false)
                };
                MONITOR_DETECTED_GAMUT.store(detected as u8, Ordering::Relaxed);
                MONITOR_HAS_ICC.store(has_icc, Ordering::Relaxed);

                if let (Some(surface), Some(cm)) = (&data.surface, state.color_management.as_ref())
                {
                    let window_id = crate::platform_impl::wayland::make_wid(surface);
                    if let Some(window) = state.windows.get_mut().get(&window_id) {
                        if let Ok(mut win) = window.lock() {
                            if let Some(win_cm) = win.color_management.as_mut() {
                                cm.sync_window_color_space(surface, win_cm, qhandle);
                            }
                        }
                        if let Some(req) = state.window_requests.get_mut().get(&window_id) {
                            req.redraw_requested.store(true, Ordering::Relaxed);
                        }
                    }
                }
            },
            _ => {},
        }
    }
}

type WpImageDescriptionInfoInfoEventAlias = WpImageDescriptionInfoV1;

delegate_dispatch!(WinitState: [WpColorManagerV1: GlobalData] => ColorManagementManager);
delegate_dispatch!(WinitState: [WpColorManagementSurfaceV1: GlobalData] => ColorManagementManager);
delegate_dispatch!(WinitState: [WpColorManagementSurfaceFeedbackV1: SurfaceFeedbackData] => ColorManagementManager);
delegate_dispatch!(WinitState: [WpImageDescriptionCreatorParamsV1: GlobalData] => ColorManagementManager);
delegate_dispatch!(WinitState: [WpImageDescriptionV1: ImageDescriptionData] => ColorManagementManager);
delegate_dispatch!(WinitState: [WpImageDescriptionInfoV1: ImageDescriptionInfoData] => ColorManagementManager);
