//! Safe Rust facade over LittleCMS 2 (`liblcms2`) for LumaPrint (`lumaprint-cms`).
//!
//! All ICC profile parsing, 16-bit color transformations (`cmsCreateTransformTHR`, `cmsDoTransform`),
//! Black Point Compensation (`cmsFLAGS_BLACKPOINTCOMPENSATION`), Softproofing (`cmsCreateProofingTransformTHR`),
//! and Gamut Warning (`cmsFLAGS_GAMUTCHECK`) are delegated to the industry-standard `liblcms2` C library.
//! All `unsafe` FFI calls are strictly isolated inside the private `lcms2` module.
#![deny(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_color::{ImageBuffer16, Rgb8, Rgb16};
use lumaprint_i18n::tr;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use thiserror::Error;

#[allow(unsafe_code)]
mod lcms2 {
    use std::ffi::{c_char, c_int, c_uint, c_void};
    use std::ptr;

    pub type CmsContext = *mut c_void;
    pub type CmsHProfile = *mut c_void;
    pub type CmsHTransform = *mut c_void;
    pub type CmsToneCurve = c_void;

    /// `TYPE_RGB_16`: `COLORSPACE_SH(PT_RGB=4) | CHANNELS_SH(3) | BYTES_SH(2)` = `(4 << 16) | (3 << 3) | 2`.
    pub const TYPE_RGB_16: u32 = (4 << 16) | (3 << 3) | 2;

    pub const INTENT_PERCEPTUAL: u32 = 0;
    pub const INTENT_RELATIVE_COLORIMETRIC: u32 = 1;
    pub const INTENT_SATURATION: u32 = 2;
    pub const INTENT_ABSOLUTE_COLORIMETRIC: u32 = 3;

    pub const CMS_FLAGS_NOCACHE: u32 = 0x0040;
    pub const CMS_FLAGS_NOOPTIMIZE: u32 = 0x0100;
    pub const CMS_FLAGS_HIGHRESPRECALC: u32 = 0x0400;
    pub const CMS_FLAGS_GAMUTCHECK: u32 = 0x1000;
    pub const CMS_FLAGS_BLACKPOINTCOMPENSATION: u32 = 0x2000;
    pub const CMS_FLAGS_SOFTPROOFING: u32 = 0x4000;

    pub const CMS_INFO_DESCRIPTION: u32 = 0;

    pub const CMS_SIG_MEDIA_WHITE_POINT_TAG: u32 = 0x7774_7074; // 'wtpt'
    pub const CMS_SIG_MEDIA_BLACK_POINT_TAG: u32 = 0x626B_7074; // 'bkpt'
    pub const CMS_SIG_RED_MATRIX_COLUMN_TAG: u32 = 0x7258_595A; // 'rXYZ'
    pub const CMS_SIG_GREEN_MATRIX_COLUMN_TAG: u32 = 0x6758_595A; // 'gXYZ'
    pub const CMS_SIG_BLUE_MATRIX_COLUMN_TAG: u32 = 0x6258_595A; // 'bXYZ'

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    pub struct CmsCIEXYZ {
        pub x: f64,
        pub y: f64,
        pub z: f64,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    pub struct CmsCIExyY {
        pub x: f64,
        pub y: f64,
        pub big_y: f64,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    pub struct CmsCIExyYTriple {
        pub red: CmsCIExyY,
        pub green: CmsCIExyY,
        pub blue: CmsCIExyY,
    }

    #[link(name = "lcms2")]
    unsafe extern "C" {
        fn cmsCreateContext(plugin: *mut c_void, user_data: *mut c_void) -> CmsContext;
        fn cmsDeleteContext(context_id: CmsContext);

        fn cmsOpenProfileFromMemTHR(
            context_id: CmsContext,
            mem_ptr: *const c_void,
            dw_size: u32,
        ) -> CmsHProfile;
        fn cmsCloseProfile(h_profile: CmsHProfile) -> c_int;
        fn cmsSaveProfileToMem(
            h_profile: CmsHProfile,
            mem_ptr: *mut c_void,
            bytes_needed: *mut u32,
        ) -> c_int;

        fn cmsCreate_sRGBProfileTHR(context_id: CmsContext) -> CmsHProfile;
        fn cmsCreateRGBProfileTHR(
            context_id: CmsContext,
            white_point: *const CmsCIExyY,
            primaries: *const CmsCIExyYTriple,
            transfer_function: *const *mut CmsToneCurve,
        ) -> CmsHProfile;

        fn cmsBuildParametricToneCurve(
            context_id: CmsContext,
            curve_type: i32,
            params: *const f64,
        ) -> *mut CmsToneCurve;
        fn cmsFreeToneCurve(curve: *mut CmsToneCurve);

        fn cmsGetProfileInfoASCII(
            h_profile: CmsHProfile,
            info: c_uint,
            language_code: *const c_char,
            country_code: *const c_char,
            buffer: *mut c_char,
            buffer_size: u32,
        ) -> u32;

        fn cmsReadTag(h_profile: CmsHProfile, sig: u32) -> *const c_void;

        fn cmsDetectDestinationBlackPoint(
            black_point: *mut CmsCIEXYZ,
            h_profile: CmsHProfile,
            intent: u32,
            dw_flags: u32,
        ) -> c_int;

        fn cmsCreateTransformTHR(
            context_id: CmsContext,
            input: CmsHProfile,
            input_format: u32,
            output: CmsHProfile,
            output_format: u32,
            intent: u32,
            dw_flags: u32,
        ) -> CmsHTransform;

        fn cmsCreateProofingTransformTHR(
            context_id: CmsContext,
            input: CmsHProfile,
            input_format: u32,
            output: CmsHProfile,
            output_format: u32,
            proofing: CmsHProfile,
            intent: u32,
            proofing_intent: u32,
            dw_flags: u32,
        ) -> CmsHTransform;

        fn cmsDeleteTransform(h_transform: CmsHTransform);

        fn cmsDoTransform(
            transform: CmsHTransform,
            input_buffer: *const c_void,
            output_buffer: *mut c_void,
            size: u32,
        );

        fn cmsSetAlarmCodesTHR(context_id: CmsContext, alarm_codes: *const u16);
    }

    /// Safe RAII wrapper around `cmsHPROFILE`.
    pub struct ProfileHandle(CmsHProfile);

    // SAFETY: `cmsHPROFILE` is only mutated during creation/closing and is thread-safe
    // when creating transforms with `THR` functions and `CMS_FLAGS_NOCACHE`.
    unsafe impl Send for ProfileHandle {}
    unsafe impl Sync for ProfileHandle {}

    impl ProfileHandle {
        pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
            let len = u32::try_from(bytes.len()).ok()?;
            let handle =
                unsafe { cmsOpenProfileFromMemTHR(ptr::null_mut(), bytes.as_ptr().cast(), len) };
            if handle.is_null() {
                None
            } else {
                Some(Self(handle))
            }
        }

        pub fn create_srgb() -> Option<Self> {
            let handle = unsafe { cmsCreate_sRGBProfileTHR(ptr::null_mut()) };
            if handle.is_null() {
                None
            } else {
                Some(Self(handle))
            }
        }

        pub fn create_parametric_rgb(
            white_point: CmsCIExyY,
            primaries: CmsCIExyYTriple,
            curve_type: i32,
            curve_params: &[f64],
        ) -> Option<Self> {
            unsafe {
                let curve =
                    cmsBuildParametricToneCurve(ptr::null_mut(), curve_type, curve_params.as_ptr());
                if curve.is_null() {
                    return None;
                }
                let curves: [*mut CmsToneCurve; 3] = [curve, curve, curve];
                let handle = cmsCreateRGBProfileTHR(
                    ptr::null_mut(),
                    &raw const white_point,
                    &raw const primaries,
                    curves.as_ptr(),
                );
                cmsFreeToneCurve(curve);
                if handle.is_null() {
                    None
                } else {
                    Some(Self(handle))
                }
            }
        }

        pub fn description(&self) -> Option<String> {
            unsafe {
                let lang = b"en\0";
                let country = b"US\0";
                let needed = cmsGetProfileInfoASCII(
                    self.0,
                    CMS_INFO_DESCRIPTION,
                    lang.as_ptr().cast(),
                    country.as_ptr().cast(),
                    ptr::null_mut(),
                    0,
                );
                if needed == 0 || needed > 4096 {
                    return None;
                }
                let mut buf = vec![0u8; needed as usize];
                let written = cmsGetProfileInfoASCII(
                    self.0,
                    CMS_INFO_DESCRIPTION,
                    lang.as_ptr().cast(),
                    country.as_ptr().cast(),
                    buf.as_mut_ptr().cast(),
                    needed,
                );
                if written == 0 {
                    return None;
                }
                let trimmed: Vec<u8> = buf.into_iter().take_while(|&b| b != 0).collect();
                let s = String::from_utf8_lossy(&trimmed).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            }
        }

        pub fn to_bytes(&self) -> Option<Vec<u8>> {
            unsafe {
                let mut needed: u32 = 0;
                if cmsSaveProfileToMem(self.0, ptr::null_mut(), &raw mut needed) == 0 || needed == 0
                {
                    return None;
                }
                let mut buf = vec![0u8; needed as usize];
                if cmsSaveProfileToMem(self.0, buf.as_mut_ptr().cast(), &raw mut needed) == 0 {
                    return None;
                }
                buf.truncate(needed as usize);
                Some(buf)
            }
        }

        pub fn read_xyz_tag(&self, sig: u32) -> Option<[f32; 3]> {
            unsafe {
                let ptr = cmsReadTag(self.0, sig) as *const CmsCIEXYZ;
                if ptr.is_null() {
                    None
                } else {
                    let xyz = *ptr;
                    Some([xyz.x as f32, xyz.y as f32, xyz.z as f32])
                }
            }
        }

        pub fn detect_destination_black_point(&self, intent: u32) -> Option<[f32; 3]> {
            unsafe {
                let mut bp = CmsCIEXYZ::default();
                let ok = cmsDetectDestinationBlackPoint(&raw mut bp, self.0, intent, 0);
                if ok != 0 {
                    Some([bp.x as f32, bp.y as f32, bp.z as f32])
                } else {
                    None
                }
            }
        }
    }

    impl Drop for ProfileHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    cmsCloseProfile(self.0);
                }
            }
        }
    }

    /// Safe RAII wrapper around `cmsHTRANSFORM` (and optional owning `cmsContext`).
    pub struct TransformHandle {
        transform: CmsHTransform,
        context: CmsContext,
    }

    // SAFETY: Created with `CMS_FLAGS_NOCACHE`, making `cmsDoTransform` completely stateless
    // and safe to invoke concurrently from multiple `rayon` worker threads.
    unsafe impl Send for TransformHandle {}
    unsafe impl Sync for TransformHandle {}

    impl TransformHandle {
        pub fn new_rgb16(
            input: &ProfileHandle,
            output: &ProfileHandle,
            intent: u32,
            bpc: bool,
        ) -> Option<Self> {
            let mut flags = CMS_FLAGS_HIGHRESPRECALC | CMS_FLAGS_NOCACHE;
            if bpc {
                flags |= CMS_FLAGS_BLACKPOINTCOMPENSATION;
            }
            let transform = unsafe {
                cmsCreateTransformTHR(
                    ptr::null_mut(),
                    input.0,
                    TYPE_RGB_16,
                    output.0,
                    TYPE_RGB_16,
                    intent,
                    flags,
                )
            };
            if transform.is_null() {
                None
            } else {
                Some(Self {
                    transform,
                    context: ptr::null_mut(),
                })
            }
        }

        pub fn new_matrix_shaper_rgb16(
            input: &ProfileHandle,
            output: &ProfileHandle,
        ) -> Option<Self> {
            let flags = CMS_FLAGS_NOOPTIMIZE | CMS_FLAGS_NOCACHE | CMS_FLAGS_BLACKPOINTCOMPENSATION;
            let transform = unsafe {
                cmsCreateTransformTHR(
                    ptr::null_mut(),
                    input.0,
                    TYPE_RGB_16,
                    output.0,
                    TYPE_RGB_16,
                    INTENT_RELATIVE_COLORIMETRIC,
                    flags,
                )
            };
            if transform.is_null() {
                None
            } else {
                Some(Self {
                    transform,
                    context: ptr::null_mut(),
                })
            }
        }

        pub fn new_proofing_rgb16(
            input: &ProfileHandle,
            display_output: &ProfileHandle,
            proofing_printer: &ProfileHandle,
            intent: u32,
            simulate_paper_white: bool,
            bpc: bool,
        ) -> Option<Self> {
            let mut flags = CMS_FLAGS_SOFTPROOFING | CMS_FLAGS_HIGHRESPRECALC | CMS_FLAGS_NOCACHE;
            if bpc {
                flags |= CMS_FLAGS_BLACKPOINTCOMPENSATION;
            }
            let proofing_intent = if simulate_paper_white {
                INTENT_ABSOLUTE_COLORIMETRIC
            } else {
                INTENT_RELATIVE_COLORIMETRIC
            };
            let transform = unsafe {
                cmsCreateProofingTransformTHR(
                    ptr::null_mut(),
                    input.0,
                    TYPE_RGB_16,
                    display_output.0,
                    TYPE_RGB_16,
                    proofing_printer.0,
                    intent,
                    proofing_intent,
                    flags,
                )
            };
            if transform.is_null() {
                None
            } else {
                Some(Self {
                    transform,
                    context: ptr::null_mut(),
                })
            }
        }

        /// Creates a proofing transform with a dedicated `cmsContext` whose alarm codes are set to
        /// `[0xDEAD, 0xBEEF, 0xCAFE, ...]` so out-of-gamut pixels can be detected via `CMS_FLAGS_GAMUTCHECK`.
        pub fn new_gamut_check_rgb16(
            input: &ProfileHandle,
            proofing_printer: &ProfileHandle,
        ) -> Option<Self> {
            unsafe {
                let ctx = cmsCreateContext(ptr::null_mut(), ptr::null_mut());
                if ctx.is_null() {
                    return None;
                }
                let mut alarms = [0u16; 16];
                alarms[0] = 0xDEAD;
                alarms[1] = 0xBEEF;
                alarms[2] = 0xCAFE;
                cmsSetAlarmCodesTHR(ctx, alarms.as_ptr());

                let flags = CMS_FLAGS_GAMUTCHECK | CMS_FLAGS_SOFTPROOFING | CMS_FLAGS_NOCACHE;
                let transform = cmsCreateProofingTransformTHR(
                    ctx,
                    input.0,
                    TYPE_RGB_16,
                    input.0,
                    TYPE_RGB_16,
                    proofing_printer.0,
                    INTENT_RELATIVE_COLORIMETRIC,
                    INTENT_RELATIVE_COLORIMETRIC,
                    flags,
                );
                if transform.is_null() {
                    cmsDeleteContext(ctx);
                    None
                } else {
                    Some(Self {
                        transform,
                        context: ctx,
                    })
                }
            }
        }

        #[inline]
        pub fn transform_pixel(&self, px: [u16; 3]) -> [u16; 3] {
            let mut out = [0u16; 3];
            unsafe {
                cmsDoTransform(
                    self.transform,
                    px.as_ptr().cast(),
                    out.as_mut_ptr().cast(),
                    1,
                );
            }
            out
        }

        /// Transforms a contiguous slice of `u16` RGB triplets (`len` must be a multiple of 3) in-place.
        #[inline]
        pub fn transform_slice_in_place(&self, rgb_u16_slice: &mut [u16]) {
            let num_pixels = rgb_u16_slice.len() / 3;
            if num_pixels == 0 {
                return;
            }
            let Ok(count) = u32::try_from(num_pixels) else {
                return;
            };
            let ptr = rgb_u16_slice.as_mut_ptr();
            unsafe {
                cmsDoTransform(self.transform, ptr.cast(), ptr.cast(), count);
            }
        }
    }

    impl Drop for TransformHandle {
        fn drop(&mut self) {
            if !self.transform.is_null() {
                unsafe {
                    cmsDeleteTransform(self.transform);
                }
            }
            if !self.context.is_null() {
                unsafe {
                    cmsDeleteContext(self.context);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CmsError {
    #[error("ICC profile is truncated (size {0} bytes)")]
    Truncated(usize),
    #[error("invalid ICC profile signature (expected 'acsp')")]
    BadSignature,
    #[error("unsupported or missing ICC LUT tag '{0}'")]
    MissingTag(String),
    #[error("invalid ICC tag structure: {0}")]
    InvalidTag(String),
    #[error("failed to read ICC file '{path}': {reason}")]
    Io { path: String, reason: String },
}

/// ICC Rendering Intent (EPL manual p. 21, 29).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RenderingIntent {
    /// Wahrnehmung (`Perceptual`, `B2A0` / `A2B0`) — recommended for photos by EPL manual p. 21.
    #[default]
    Perceptual,
    /// Relative colorimetric (`Relative Colorimetric`, `B2A1` / `A2B1`).
    RelativeColorimetric,
    /// Sättigung (`Saturation`, `B2A2` / `A2B2`).
    Saturation,
    /// Absolute colorimetric (`Absolute Colorimetric`, `B2A1` without media white shift).
    AbsoluteColorimetric,
}

impl RenderingIntent {
    pub const ALL: [Self; 4] = [
        Self::Perceptual,
        Self::RelativeColorimetric,
        Self::Saturation,
        Self::AbsoluteColorimetric,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Perceptual => tr!("Perceptual"),
            Self::RelativeColorimetric => tr!("Relative colorimetric"),
            Self::Saturation => tr!("Saturation"),
            Self::AbsoluteColorimetric => tr!("Absolute colorimetric"),
        }
    }

    pub fn b2a_tag_sig(self) -> &'static [u8; 4] {
        match self {
            Self::Perceptual => b"B2A0",
            Self::RelativeColorimetric | Self::AbsoluteColorimetric => b"B2A1",
            Self::Saturation => b"B2A2",
        }
    }

    pub fn a2b_tag_sig(self) -> &'static [u8; 4] {
        match self {
            Self::Perceptual => b"A2B0",
            Self::RelativeColorimetric | Self::AbsoluteColorimetric => b"A2B1",
            Self::Saturation => b"A2B2",
        }
    }

    fn lcms_intent(self) -> u32 {
        match self {
            Self::Perceptual => lcms2::INTENT_PERCEPTUAL,
            Self::RelativeColorimetric => lcms2::INTENT_RELATIVE_COLORIMETRIC,
            Self::Saturation => lcms2::INTENT_SATURATION,
            Self::AbsoluteColorimetric => lcms2::INTENT_ABSOLUTE_COLORIMETRIC,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Perceptual => 0,
            Self::RelativeColorimetric => 1,
            Self::Saturation => 2,
            Self::AbsoluteColorimetric => 3,
        }
    }
}

/// Builtin source RGB color spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WorkingColorSpace {
    #[default]
    Srgb,
    AdobeRgb1998,
    ProPhotoRgb,
    DisplayP3,
}

impl WorkingColorSpace {
    pub const ALL: [Self; 4] = [
        Self::Srgb,
        Self::AdobeRgb1998,
        Self::ProPhotoRgb,
        Self::DisplayP3,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB IEC61966-2.1",
            Self::AdobeRgb1998 => "Adobe RGB (1998)",
            Self::ProPhotoRgb => "ProPhoto RGB (ROMM)",
            Self::DisplayP3 => "Display P3",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Srgb => 0,
            Self::AdobeRgb1998 => 1,
            Self::ProPhotoRgb => 2,
            Self::DisplayP3 => 3,
        }
    }

    fn lcms_profile(self) -> Option<&'static lcms2::ProfileHandle> {
        static SRGB: OnceLock<Option<lcms2::ProfileHandle>> = OnceLock::new();
        static ADOBE_RGB: OnceLock<Option<lcms2::ProfileHandle>> = OnceLock::new();
        static PROPHOTO_RGB: OnceLock<Option<lcms2::ProfileHandle>> = OnceLock::new();
        static DISPLAY_P3: OnceLock<Option<lcms2::ProfileHandle>> = OnceLock::new();

        match self {
            Self::Srgb => SRGB.get_or_init(lcms2::ProfileHandle::create_srgb).as_ref(),
            Self::AdobeRgb1998 => ADOBE_RGB
                .get_or_init(|| {
                    let wp = lcms2::CmsCIExyY {
                        x: 0.3127,
                        y: 0.3290,
                        big_y: 1.0,
                    };
                    let primaries = lcms2::CmsCIExyYTriple {
                        red: lcms2::CmsCIExyY {
                            x: 0.6400,
                            y: 0.3300,
                            big_y: 1.0,
                        },
                        green: lcms2::CmsCIExyY {
                            x: 0.2100,
                            y: 0.7100,
                            big_y: 1.0,
                        },
                        blue: lcms2::CmsCIExyY {
                            x: 0.1500,
                            y: 0.0600,
                            big_y: 1.0,
                        },
                    };
                    lcms2::ProfileHandle::create_parametric_rgb(wp, primaries, 1, &[563.0 / 256.0])
                })
                .as_ref(),
            Self::ProPhotoRgb => PROPHOTO_RGB
                .get_or_init(|| {
                    let wp = lcms2::CmsCIExyY {
                        x: 0.34567,
                        y: 0.35850,
                        big_y: 1.0,
                    };
                    let primaries = lcms2::CmsCIExyYTriple {
                        red: lcms2::CmsCIExyY {
                            x: 0.7347,
                            y: 0.2653,
                            big_y: 1.0,
                        },
                        green: lcms2::CmsCIExyY {
                            x: 0.1596,
                            y: 0.8404,
                            big_y: 1.0,
                        },
                        blue: lcms2::CmsCIExyY {
                            x: 0.0366,
                            y: 0.0001,
                            big_y: 1.0,
                        },
                    };
                    lcms2::ProfileHandle::create_parametric_rgb(
                        wp,
                        primaries,
                        4,
                        &[1.8, 1.0, 0.0, 1.0 / 16.0, 0.03125],
                    )
                })
                .as_ref(),
            Self::DisplayP3 => DISPLAY_P3
                .get_or_init(|| {
                    let wp = lcms2::CmsCIExyY {
                        x: 0.3127,
                        y: 0.3290,
                        big_y: 1.0,
                    };
                    let primaries = lcms2::CmsCIExyYTriple {
                        red: lcms2::CmsCIExyY {
                            x: 0.6800,
                            y: 0.3200,
                            big_y: 1.0,
                        },
                        green: lcms2::CmsCIExyY {
                            x: 0.2650,
                            y: 0.6900,
                            big_y: 1.0,
                        },
                        blue: lcms2::CmsCIExyY {
                            x: 0.1500,
                            y: 0.0600,
                            big_y: 1.0,
                        },
                    };
                    lcms2::ProfileHandle::create_parametric_rgb(
                        wp,
                        primaries,
                        4,
                        &[2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045],
                    )
                })
                .as_ref(),
        }
    }

    /// Serializes this builtin working space as an ICC profile (generated by `liblcms2`).
    pub fn icc_profile_bytes(self) -> Option<Vec<u8>> {
        self.lcms_profile()?.to_bytes()
    }

    /// Chromaticities `[x, y]` of the D50-adapted red, green and blue primaries.
    fn primaries_xy_d50(self) -> Option<[[f32; 2]; 3]> {
        Some([
            xyz_to_xy(self.linear_rgb_to_xyz_d50(1.0, 0.0, 0.0))?,
            xyz_to_xy(self.linear_rgb_to_xyz_d50(0.0, 1.0, 0.0))?,
            xyz_to_xy(self.linear_rgb_to_xyz_d50(0.0, 0.0, 1.0))?,
        ])
    }

    /// Converts normalized linear `[r, g, b]` (`0.0..=1.0`) to CIE XYZ (D50 PCS).
    pub fn linear_rgb_to_xyz_d50(self, r: f32, g: f32, b: f32) -> [f32; 3] {
        // Bradford-adapted 3x3 matrices to D50 PCS (X=0.9642, Y=1.0000, Z=0.8249)
        let m: [[f32; 3]; 3] = match self {
            Self::Srgb => [
                [0.4360747, 0.3850649, 0.1430804],
                [0.2225045, 0.7168786, 0.0606169],
                [0.0139322, 0.0971045, 0.7141733],
            ],
            Self::AdobeRgb1998 => [
                [0.609_755_9, 0.205_240_1, 0.149_224],
                [0.311_124_2, 0.625_656, 0.063_219_7],
                [0.019_481_1, 0.060_890_2, 0.744_838_7],
            ],
            Self::ProPhotoRgb => [
                [0.797_674_9, 0.135_191_7, 0.031_353_4],
                [0.288_040_2, 0.711_874_1, 0.000_085_7],
                [0.0, 0.0, 0.825_21],
            ],
            Self::DisplayP3 => [
                [0.515_102, 0.291_965, 0.157_153],
                [0.241_182, 0.692_236, 0.066_581_9],
                [-0.001_049_4, 0.041_881_8, 0.784_378],
            ],
        };
        [
            m[0][0] * r + m[0][1] * g + m[0][2] * b,
            m[1][0] * r + m[1][1] * g + m[1][2] * b,
            m[2][0] * r + m[2][1] * g + m[2][2] * b,
        ]
    }

    /// Decodes encoded `[r, g, b]` (`0.0..=1.0`) to linear light (`0.0..=1.0`).
    pub fn to_linear(self, c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        match self {
            Self::Srgb | Self::DisplayP3 => {
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            }
            Self::AdobeRgb1998 => c.powf(563.0 / 256.0),
            Self::ProPhotoRgb => {
                if c <= 0.03125 {
                    c / 16.0
                } else {
                    c.powf(1.8)
                }
            }
        }
    }

    /// Encodes linear light (`0.0..=1.0`) to sRGB gamma (`0.0..=1.0`).
    pub fn linear_to_srgb(c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    }

    /// Encodes linear light (`0.0..=1.0`) into this color space's transfer curve (`0.0..=1.0`).
    pub fn from_linear(self, c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        match self {
            Self::Srgb | Self::DisplayP3 => Self::linear_to_srgb(c),
            Self::AdobeRgb1998 => c.powf(256.0 / 563.0),
            Self::ProPhotoRgb => {
                if c <= 0.001_953_125 {
                    c * 16.0
                } else {
                    c.powf(1.0 / 1.8)
                }
            }
        }
    }

    /// Converts CIE XYZ (D50 PCS) to normalized linear `[r, g, b]` in this color space.
    pub fn xyz_d50_to_linear_rgb(self, xyz: [f32; 3]) -> [f32; 3] {
        // Exact inverse 3x3 matrices of `linear_rgb_to_xyz_d50`
        let inv: [[f32; 3]; 3] = match self {
            Self::Srgb => [
                [3.133_856, -1.616_866_7, -0.490_614_6],
                [-0.978_768_4, 1.916_141_5, 0.033_454],
                [0.071_945_3, -0.228_991_4, 1.405_242_7],
            ],
            Self::AdobeRgb1998 => [
                [1.962_427_4, -0.610_534_3, -0.341_340_4],
                [-0.978_768_4, 1.916_141_5, 0.033_454],
                [0.028_686_9, -0.140_675_2, 1.348_765_5],
            ],
            Self::ProPhotoRgb => [
                [1.345_943_3, -0.255_607_5, -0.051_111_8],
                [-0.544_598_9, 1.508_167_3, 0.020_535_1],
                [0.0, 0.0, 1.211_812_8],
            ],
            Self::DisplayP3 => [
                [2.404_147_7, -0.989_898_7, -0.397_631_7],
                [-0.842_228_4, 1.798_850_5, 0.016_048_2],
                [0.048_187_1, -0.097_373_9, 1.273_506_6],
            ],
        };
        [
            inv[0][0] * xyz[0] + inv[0][1] * xyz[1] + inv[0][2] * xyz[2],
            inv[1][0] * xyz[0] + inv[1][1] * xyz[1] + inv[1][2] * xyz[2],
            inv[2][0] * xyz[0] + inv[2][1] * xyz[1] + inv[2][2] * xyz[2],
        ]
    }

    /// Converts CIE XYZ (D50 PCS) to 16-bit `Rgb16` encoded in this color space.
    pub fn xyz_d50_to_rgb16(self, xyz: [f32; 3]) -> Rgb16 {
        let [rl, gl, bl] = self.xyz_d50_to_linear_rgb(xyz);
        let r = (self.from_linear(rl) * 65535.0).round() as u16;
        let g = (self.from_linear(gl) * 65535.0).round() as u16;
        let b = (self.from_linear(bl) * 65535.0).round() as u16;
        Rgb16 { r, g, b }
    }

    /// Converts `[r, g, b]` (`0..=65535`) to CIE XYZ (D50 PCS).
    pub fn rgb16_to_xyz_d50(self, r: u16, g: u16, b: u16) -> [f32; 3] {
        let rl = self.to_linear(f32::from(r) / 65535.0);
        let gl = self.to_linear(f32::from(g) / 65535.0);
        let bl = self.to_linear(f32::from(b) / 65535.0);
        self.linear_rgb_to_xyz_d50(rl, gl, bl)
    }

    /// Converts a single 16-bit pixel from `self` into `dst_space` via CIE XYZ (D50).
    pub fn convert_pixel_to(self, px: Rgb16, dst_space: Self) -> Rgb16 {
        if self == dst_space {
            return px;
        }
        let xyz = self.rgb16_to_xyz_d50(px.r, px.g, px.b);
        dst_space.xyz_d50_to_rgb16(xyz)
    }

    /// Converts a 16-bit image buffer in-place from `self` into `dst_space`.
    pub fn convert_buffer_to(self, buf: &mut ImageBuffer16, dst_space: Self) {
        if self == dst_space {
            return;
        }
        buf.data.par_chunks_exact_mut(3).for_each(|px| {
            let xyz = self.rgb16_to_xyz_d50(px[0], px[1], px[2]);
            let out = dst_space.xyz_d50_to_rgb16(xyz);
            px[0] = out.r;
            px[1] = out.g;
            px[2] = out.b;
        });
    }
}

/// Standard ICC D50 PCS illuminant XYZ (matches the row sums of sRGB/AdobeRGB/ProPhoto/DisplayP3 D50 matrices).
pub const D50_XYZ: [f32; 3] = [0.96422, 1.00000, 0.82521];

/// Converts CIE XYZ (D50) to CIE `L*a*b*` (`L` in `0..=100`, `a` and `b` in `-128..=127`).
pub fn xyz_d50_to_lab(xyz: [f32; 3]) -> [f32; 3] {
    fn f(t: f32) -> f32 {
        const DELTA: f32 = 6.0 / 29.0;
        const DELTA3: f32 = DELTA * DELTA * DELTA;
        if t > DELTA3 {
            t.cbrt()
        } else {
            t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
        }
    }
    let fx = f((xyz[0] / D50_XYZ[0]).max(0.0));
    let fy = f((xyz[1] / D50_XYZ[1]).max(0.0));
    let fz = f((xyz[2] / D50_XYZ[2]).max(0.0));
    let l = (116.0 * fy - 16.0).clamp(0.0, 100.0);
    let a = (500.0 * (fx - fy)).clamp(-128.0, 127.0);
    let b = (200.0 * (fy - fz)).clamp(-128.0, 127.0);
    [l, a, b]
}

/// Converts CIE `L*a*b*` to CIE XYZ (D50).
pub fn lab_to_xyz_d50(lab: [f32; 3]) -> [f32; 3] {
    fn finv(t: f32) -> f32 {
        const DELTA: f32 = 6.0 / 29.0;
        if t > DELTA {
            t * t * t
        } else {
            3.0 * DELTA * DELTA * (t - 4.0 / 29.0)
        }
    }
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    [
        D50_XYZ[0] * finv(fx),
        D50_XYZ[1] * finv(fy),
        D50_XYZ[2] * finv(fz),
    ]
}

/// Converts CIE XYZ (D50) to sRGB `[u16; 3]`.
pub fn xyz_d50_to_srgb16(xyz: [f32; 3]) -> Rgb16 {
    WorkingColorSpace::Srgb.xyz_d50_to_rgb16(xyz)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ProofKey {
    src_space: WorkingColorSpace,
    dst_display_space: WorkingColorSpace,
    intent: RenderingIntent,
    bpc: bool,
    simulate_paper_white: bool,
}

struct IccProfileInner {
    handle: lcms2::ProfileHandle,
    /// Cached device transforms indexed by `(src_space.index() * 8) + (intent.index() * 2) + usize::from(bpc)`.
    device_transforms: [OnceLock<Option<lcms2::TransformHandle>>; 32],
    /// Cached gamut-check transforms indexed by `src_space.index()`.
    gamut_transforms: [OnceLock<Option<lcms2::TransformHandle>>; 4],
    /// Cached softproofing transforms (`src_space -> printer -> dst_display_space`).
    proof_transforms: RwLock<HashMap<ProofKey, Arc<lcms2::TransformHandle>>>,
}

impl std::fmt::Debug for IccProfileInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IccProfileInner").finish_non_exhaustive()
    }
}

/// Parsed ICC v2/v4 Printer / Output / Input Profile backed by `liblcms2`.
#[derive(Debug, Clone)]
pub struct IccProfile {
    pub description: String,
    pub file_path: Option<PathBuf>,
    pub media_white_xyz: [f32; 3],
    pub media_black_xyz: [f32; 3],
    pub red_primary_xyz: Option<[f32; 3]>,
    inner: Arc<IccProfileInner>,
}

impl IccProfile {
    /// Parses an ICC profile from raw bytes using `liblcms2`.
    pub fn from_bytes(bytes: &[u8], file_path: Option<PathBuf>) -> Result<Self, CmsError> {
        if bytes.len() < 132 {
            return Err(CmsError::Truncated(bytes.len()));
        }
        if bytes.get(36..40) != Some(b"acsp") {
            return Err(CmsError::BadSignature);
        }
        let declared_len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let icc_slice = if declared_len >= 132 && declared_len <= bytes.len() {
            &bytes[..declared_len]
        } else {
            bytes
        };

        let handle = lcms2::ProfileHandle::from_bytes(icc_slice)
            .ok_or_else(|| CmsError::InvalidTag("liblcms2 rejected ICC profile".into()))?;

        let description = handle
            .description()
            .or_else(|| fallback_desc_from_bytes(icc_slice))
            .unwrap_or_else(|| String::from("ICC Profile"));

        let media_white_xyz = handle
            .read_xyz_tag(lcms2::CMS_SIG_MEDIA_WHITE_POINT_TAG)
            .unwrap_or(D50_XYZ);

        let media_black_xyz = handle
            .detect_destination_black_point(lcms2::INTENT_RELATIVE_COLORIMETRIC)
            .or_else(|| handle.read_xyz_tag(lcms2::CMS_SIG_MEDIA_BLACK_POINT_TAG))
            .unwrap_or([0.003, 0.003, 0.0025]);

        let red_primary_xyz = handle.read_xyz_tag(lcms2::CMS_SIG_RED_MATRIX_COLUMN_TAG);

        let inner = Arc::new(IccProfileInner {
            handle,
            device_transforms: std::array::from_fn(|_| OnceLock::new()),
            gamut_transforms: std::array::from_fn(|_| OnceLock::new()),
            proof_transforms: RwLock::new(HashMap::new()),
        });

        Ok(Self {
            description,
            file_path,
            media_white_xyz,
            media_black_xyz,
            red_primary_xyz,
            inner,
        })
    }

    /// Loads an ICC profile from disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, CmsError> {
        let p = path.as_ref();
        let bytes = std::fs::read(p).map_err(|e| CmsError::Io {
            path: p.display().to_string(),
            reason: e.to_string(),
        })?;
        Self::from_bytes(&bytes, Some(p.to_path_buf()))
    }

    /// Converts a 16-bit image buffer from this embedded input/display ICC profile into `dst_space`
    /// using `liblcms2` (`INTENT_RELATIVE_COLORIMETRIC` + Black Point Compensation).
    pub fn convert_buffer_to_working_space(
        &self,
        buf: &mut ImageBuffer16,
        dst_space: WorkingColorSpace,
    ) -> bool {
        let Some(dst_prof) = dst_space.lcms_profile() else {
            return false;
        };
        let Some(xform) =
            lcms2::TransformHandle::new_matrix_shaper_rgb16(&self.inner.handle, dst_prof)
        else {
            return false;
        };
        buf.data.par_chunks_mut(3 * 1024).for_each(|chunk| {
            xform.transform_slice_in_place(chunk);
        });
        true
    }

    /// Converts a buffer tagged with this embedded RGB profile into the smallest builtin working space
    /// whose gamut encloses the profile's colorants (ProPhoto RGB for LUT-based profiles without colorant
    /// tags). The profile's real primaries and tone curves are used, never its description, so e.g. a
    /// linear-gamma "sRGB" profile or "Adobe Wide Gamut RGB" is converted correctly. If the profile is
    /// numerically equivalent to the chosen working space the buffer is left untouched.
    ///
    /// Returns `None` (buffer untouched) if `liblcms2` cannot build an RGB transform, e.g. for gray or CMYK profiles.
    pub fn convert_embedded_buffer_to_working_space(
        &self,
        buf: &mut ImageBuffer16,
    ) -> Option<WorkingColorSpace> {
        let target = self.enclosing_working_space();
        let dst_prof = target.lcms_profile()?;
        let xform = lcms2::TransformHandle::new_matrix_shaper_rgb16(&self.inner.handle, dst_prof)?;
        if !transform_is_near_identity(&xform, target) {
            buf.data.par_chunks_mut(3 * 1024).for_each(|chunk| {
                xform.transform_slice_in_place(chunk);
            });
        }
        Some(target)
    }

    fn enclosing_working_space(&self) -> WorkingColorSpace {
        let handle = &self.inner.handle;
        let colorants = [
            lcms2::CMS_SIG_RED_MATRIX_COLUMN_TAG,
            lcms2::CMS_SIG_GREEN_MATRIX_COLUMN_TAG,
            lcms2::CMS_SIG_BLUE_MATRIX_COLUMN_TAG,
        ]
        .map(|sig| handle.read_xyz_tag(sig).and_then(xyz_to_xy));
        let [Some(r), Some(g), Some(b)] = colorants else {
            return WorkingColorSpace::ProPhotoRgb;
        };
        [
            WorkingColorSpace::Srgb,
            WorkingColorSpace::DisplayP3,
            WorkingColorSpace::AdobeRgb1998,
        ]
        .into_iter()
        .find(|ws| {
            ws.primaries_xy_d50()
                .is_some_and(|tri| [r, g, b].iter().all(|&p| triangle_contains(tri, p)))
        })
        .unwrap_or(WorkingColorSpace::ProPhotoRgb)
    }

    fn device_transform(
        &self,
        src_space: WorkingColorSpace,
        intent: RenderingIntent,
        black_point_compensation: bool,
    ) -> Option<&lcms2::TransformHandle> {
        let slot =
            (src_space.index() * 8) + (intent.index() * 2) + usize::from(black_point_compensation);
        self.inner
            .device_transforms
            .get(slot)?
            .get_or_init(|| {
                let src_prof = src_space.lcms_profile()?;
                lcms2::TransformHandle::new_rgb16(
                    src_prof,
                    &self.inner.handle,
                    intent.lcms_intent(),
                    black_point_compensation,
                )
            })
            .as_ref()
    }

    fn gamut_transform(&self, src_space: WorkingColorSpace) -> Option<&lcms2::TransformHandle> {
        let slot = src_space.index();
        self.inner
            .gamut_transforms
            .get(slot)?
            .get_or_init(|| {
                let src_prof = src_space.lcms_profile()?;
                lcms2::TransformHandle::new_gamut_check_rgb16(src_prof, &self.inner.handle)
            })
            .as_ref()
    }

    fn proof_transform(&self, key: ProofKey) -> Option<Arc<lcms2::TransformHandle>> {
        if let Ok(guard) = self.inner.proof_transforms.read() {
            if let Some(found) = guard.get(&key) {
                return Some(Arc::clone(found));
            }
        }
        let src_prof = key.src_space.lcms_profile()?;
        let dst_prof = key.dst_display_space.lcms_profile()?;
        let created = Arc::new(lcms2::TransformHandle::new_proofing_rgb16(
            src_prof,
            dst_prof,
            &self.inner.handle,
            key.intent.lcms_intent(),
            key.simulate_paper_white,
            key.bpc,
        )?);
        if let Ok(mut guard) = self.inner.proof_transforms.write() {
            guard.insert(key, Arc::clone(&created));
        }
        Some(created)
    }

    /// Transforms a single 16-bit source RGB pixel into printer device RGB (`0..=65535`) via `liblcms2`.
    pub fn transform_pixel(
        &self,
        src_space: WorkingColorSpace,
        px: Rgb16,
        intent: RenderingIntent,
        black_point_compensation: bool,
    ) -> Rgb16 {
        let Some(xform) = self.device_transform(src_space, intent, black_point_compensation) else {
            return px;
        };
        let [r, g, b] = xform.transform_pixel([px.r, px.g, px.b]);
        Rgb16 { r, g, b }
    }

    /// Checks whether a source pixel is out of the printer profile's gamut using `liblcms2` (`cmsFLAGS_GAMUTCHECK`).
    pub fn is_out_of_gamut(&self, src_space: WorkingColorSpace, px: Rgb16) -> bool {
        let Some(xform) = self.gamut_transform(src_space) else {
            return false;
        };
        let out = xform.transform_pixel([px.r, px.g, px.b]);
        out == [0xDEAD, 0xBEEF, 0xCAFE]
    }

    /// Transforms an entire 16-bit image buffer in-place using `liblcms2` across `rayon` threads.
    pub fn transform_buffer_in_place(
        &self,
        src_space: WorkingColorSpace,
        buf: &mut ImageBuffer16,
        intent: RenderingIntent,
        black_point_compensation: bool,
    ) {
        let Some(xform) = self.device_transform(src_space, intent, black_point_compensation) else {
            return;
        };
        // Process in chunks of 1024 pixels (3072 u16 values) per rayon task so `cmsDoTransform`
        // amortizes function call overhead across scanlines.
        buf.data.par_chunks_mut(3 * 1024).for_each(|chunk| {
            xform.transform_slice_in_place(chunk);
        });
    }
}

fn xyz_to_xy(xyz: [f32; 3]) -> Option<[f32; 2]> {
    let sum = xyz[0] + xyz[1] + xyz[2];
    (sum > 1e-6).then(|| [xyz[0] / sum, xyz[1] / sum])
}

/// Point-in-triangle test on xy chromaticities, tolerating primaries that lie on (or within
/// rounding distance of) the triangle's edges and vertices.
fn triangle_contains(tri: [[f32; 2]; 3], p: [f32; 2]) -> bool {
    const TOLERANCE_XY: f32 = 0.003;
    let cross = |a: [f32; 2], b: [f32; 2], q: [f32; 2]| {
        (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0])
    };
    let [v0, v1, v2] = tri;
    let orientation = cross(v0, v1, v2).signum();
    [(v0, v1), (v1, v2), (v2, v0)].into_iter().all(|(a, b)| {
        let edge_len = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-6);
        orientation * cross(a, b, p) / edge_len >= -TOLERANCE_XY
    })
}

/// Samples a 17³ RGB grid and reports whether `xform` (into `dst_space`) changes no channel by more
/// than `1e-4` in linear light, i.e. whether source and destination profile are numerically equivalent.
/// Comparing in linear light ignores the s15.16 matrix rounding that a gamma curve amplifies near black.
fn transform_is_near_identity(
    xform: &lcms2::TransformHandle,
    dst_space: WorkingColorSpace,
) -> bool {
    const STEPS: u32 = 16;
    const TOLERANCE_LINEAR: f32 = 1e-4;
    let level = |i: u32| u16::try_from(i * 65_535 / STEPS).unwrap_or(u16::MAX);
    let linear = |v: u16| dst_space.to_linear(f32::from(v) / 65_535.0);
    (0..=STEPS).all(|r| {
        (0..=STEPS).all(|g| {
            (0..=STEPS).all(|b| {
                let px = [level(r), level(g), level(b)];
                let out = xform.transform_pixel(px);
                px.iter()
                    .zip(out)
                    .all(|(&i, o)| (linear(i) - linear(o)).abs() <= TOLERANCE_LINEAR)
            })
        })
    })
}

/// Ambient viewing light temperature for softproofing (`Ref.Light` concept from TurboPrint Composer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ViewingLight {
    /// D50 (5000K Graphic Arts Standard).
    #[default]
    D50,
    /// D65 (6500K Daylight / Monitor Standard).
    D65,
    /// Warm Gallery / Halogen / Museum lighting (3200K).
    GalleryWarm3200K,
}

impl ViewingLight {
    pub const ALL: [Self; 3] = [Self::D50, Self::D65, Self::GalleryWarm3200K];

    pub fn label(self) -> &'static str {
        match self {
            Self::D50 => tr!("D50 standard light (5000K)"),
            Self::D65 => tr!("D65 daylight (6500K)"),
            Self::GalleryWarm3200K => tr!("Warm gallery light (3200K)"),
        }
    }

    pub fn white_tint_multipliers(self) -> [f32; 3] {
        match self {
            Self::D50 => [1.0, 0.99, 0.96],
            Self::D65 => [0.98, 1.0, 1.02],
            Self::GalleryWarm3200K => [1.04, 0.96, 0.86],
        }
    }
}

/// Softproof and Gamut Warning configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftproofConfig {
    pub enabled: bool,
    pub simulate_paper_white: bool,
    pub viewing_light: ViewingLight,
    pub gamut_warning: bool,
    pub gamut_warning_color: Rgb8,
}

impl Default for SoftproofConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            simulate_paper_white: true,
            viewing_light: ViewingLight::D50,
            gamut_warning: false,
            gamut_warning_color: Rgb8::new(255, 0, 200),
        }
    }
}

impl IccProfile {
    /// Applies a softproof simulation (and optional Out-of-Gamut warning overlay) to `buf`,
    /// targeting standard `sRGB` display encoding.
    pub fn apply_softproof(
        &self,
        src_space: WorkingColorSpace,
        buf: &mut ImageBuffer16,
        intent: RenderingIntent,
        bpc: bool,
        proof: SoftproofConfig,
    ) {
        self.apply_softproof_to_display(
            src_space,
            WorkingColorSpace::Srgb,
            buf,
            intent,
            bpc,
            proof,
        );
    }

    /// Applies a softproof simulation (and optional Out-of-Gamut warning overlay) to `buf`
    /// using `liblcms2` (`cmsCreateProofingTransformTHR`), encoding the final simulated print
    /// appearance into `dst_display_space` (`DisplayP3`, `AdobeRgb1998`, or `Srgb`).
    pub fn apply_softproof_to_display(
        &self,
        src_space: WorkingColorSpace,
        dst_display_space: WorkingColorSpace,
        buf: &mut ImageBuffer16,
        intent: RenderingIntent,
        bpc: bool,
        proof: SoftproofConfig,
    ) {
        if !proof.enabled && !proof.gamut_warning {
            src_space.convert_buffer_to(buf, dst_display_space);
            return;
        }
        let warn_rgb16 = WorkingColorSpace::Srgb
            .convert_pixel_to(proof.gamut_warning_color.to_rgb16(), dst_display_space);
        let light_mult = proof.viewing_light.white_tint_multipliers();
        let proof_xform = if proof.enabled {
            self.proof_transform(ProofKey {
                src_space,
                dst_display_space,
                intent,
                bpc,
                simulate_paper_white: proof.simulate_paper_white,
            })
        } else {
            None
        };

        buf.data.par_chunks_exact_mut(3).for_each(|px| {
            let src_px = Rgb16::new(px[0], px[1], px[2]);
            if proof.gamut_warning && self.is_out_of_gamut(src_space, src_px) {
                px[0] = warn_rgb16.r;
                px[1] = warn_rgb16.g;
                px[2] = warn_rgb16.b;
                return;
            }
            if !proof.enabled {
                let out = src_space.convert_pixel_to(src_px, dst_display_space);
                px[0] = out.r;
                px[1] = out.g;
                px[2] = out.b;
                return;
            }
            if let Some(ref xform) = proof_xform {
                let [r, g, b] = xform.transform_pixel([src_px.r, src_px.g, src_px.b]);
                if proof.viewing_light == ViewingLight::D65 && !proof.simulate_paper_white {
                    px[0] = r;
                    px[1] = g;
                    px[2] = b;
                } else {
                    let rl = dst_display_space.to_linear(f32::from(r) / 65535.0) * light_mult[0];
                    let gl = dst_display_space.to_linear(f32::from(g) / 65535.0) * light_mult[1];
                    let bl = dst_display_space.to_linear(f32::from(b) / 65535.0) * light_mult[2];
                    px[0] = (dst_display_space.from_linear(rl) * 65535.0).round() as u16;
                    px[1] = (dst_display_space.from_linear(gl) * 65535.0).round() as u16;
                    px[2] = (dst_display_space.from_linear(bl) * 65535.0).round() as u16;
                }
            } else {
                let out = self.transform_pixel(src_space, src_px, intent, bpc);
                px[0] = out.r;
                px[1] = out.g;
                px[2] = out.b;
            }
        });
    }
}

fn fallback_desc_from_bytes(bytes: &[u8]) -> Option<String> {
    let tag_count = u32::from_be_bytes(bytes.get(128..132)?.try_into().ok()?) as usize;
    if tag_count > 256 {
        return None;
    }
    for i in 0..tag_count {
        let entry_off = 132 + i * 12;
        let sig = bytes.get(entry_off..entry_off + 4)?;
        if sig != b"desc" {
            continue;
        }
        let off =
            u32::from_be_bytes(bytes.get(entry_off + 4..entry_off + 8)?.try_into().ok()?) as usize;
        let len =
            u32::from_be_bytes(bytes.get(entry_off + 8..entry_off + 12)?.try_into().ok()?) as usize;
        let tag = bytes.get(off..off.saturating_add(len))?;
        if tag.get(0..4) == Some(b"desc") {
            let count = u32::from_be_bytes(tag.get(8..12)?.try_into().ok()?) as usize;
            let end = 12usize.saturating_add(count).min(tag.len());
            let slice = tag.get(12..end)?;
            let trimmed: Vec<u8> = slice.iter().copied().take_while(|&b| b != 0).collect();
            return String::from_utf8(trimmed).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xyz_and_lab_d50_roundtrip_accurately() {
        let white_lab = xyz_d50_to_lab(D50_XYZ);
        assert!((white_lab[0] - 100.0).abs() < 1e-3);
        assert!(white_lab[1].abs() < 1e-3);
        assert!(white_lab[2].abs() < 1e-3);

        let xyz_back = lab_to_xyz_d50([50.0, 20.0, -30.0]);
        let lab_back = xyz_d50_to_lab(xyz_back);
        assert!((lab_back[0] - 50.0).abs() < 1e-2);
        assert!((lab_back[1] - 20.0).abs() < 1e-2);
        assert!((lab_back[2] - (-30.0)).abs() < 1e-2);
    }

    #[test]
    fn working_color_spaces_roundtrip_xyz_d50_and_preserve_wide_gamut() {
        let sample = Rgb16::new(12_345, 45_678, 34_567);
        for space in WorkingColorSpace::ALL {
            let xyz = space.rgb16_to_xyz_d50(sample.r, sample.g, sample.b);
            let back = space.xyz_d50_to_rgb16(xyz);
            assert!(
                (i32::from(back.r) - i32::from(sample.r)).abs() <= 10,
                "{space:?} R mismatch: {} vs {}",
                back.r,
                sample.r
            );
            assert!(
                (i32::from(back.g) - i32::from(sample.g)).abs() <= 10,
                "{space:?} G mismatch: {} vs {}",
                back.g,
                sample.g
            );
            assert!(
                (i32::from(back.b) - i32::from(sample.b)).abs() <= 10,
                "{space:?} B mismatch: {} vs {}",
                back.b,
                sample.b
            );
        }

        // Saturated emerald/cyan inside AdobeRGB (R=4000, G=58000, B=50000) exceeds sRGB (linear R < 0 in sRGB)
        let wide_emerald_adobe = Rgb16::new(4_000, 58_000, 50_000);
        let xyz_emerald = WorkingColorSpace::AdobeRgb1998.rgb16_to_xyz_d50(
            wide_emerald_adobe.r,
            wide_emerald_adobe.g,
            wide_emerald_adobe.b,
        );
        // In sRGB, linear R is negative and gets clipped to 0:
        let srgb_linear = WorkingColorSpace::Srgb.xyz_d50_to_linear_rgb(xyz_emerald);
        assert!(
            srgb_linear[0] < 0.0,
            "Expected wide emerald to be outside sRGB (linear R = {})",
            srgb_linear[0]
        );
        let srgb_clipped = WorkingColorSpace::Srgb.xyz_d50_to_rgb16(xyz_emerald);
        assert_eq!(srgb_clipped.r, 0);

        // In AdobeRGB1998, it stays unclipped at R ~ 4000:
        let adobe_unclipped = WorkingColorSpace::AdobeRgb1998.xyz_d50_to_rgb16(xyz_emerald);
        assert!((i32::from(adobe_unclipped.r) - 4_000).abs() <= 10);
    }

    #[test]
    fn parses_real_epson_sc_p900_icc_profiles_and_transforms_16bit_via_lcms2() {
        let icc_path =
            Path::new("/usr/share/color/icc/epson-scp900/Epson_SC-P900_700_EpsonPremiumGlossy.icc");
        if !icc_path.exists() {
            return;
        }
        let prof = IccProfile::from_file(icc_path).expect("valid Epson SC-P900 ICC profile");
        assert!(
            prof.description.contains("SC-P900") || prof.description.contains("Glossy"),
            "Description was: {}",
            prof.description
        );

        // Transform pure white & pure black
        let white_out = prof.transform_pixel(
            WorkingColorSpace::Srgb,
            Rgb16::WHITE,
            RenderingIntent::RelativeColorimetric,
            true,
        );
        assert_eq!(white_out, Rgb16::WHITE);

        let black_out = prof.transform_pixel(
            WorkingColorSpace::Srgb,
            Rgb16::BLACK,
            RenderingIntent::RelativeColorimetric,
            true,
        );
        assert!(black_out.r < 4000 && black_out.g < 4000 && black_out.b < 4000);

        // Gamut check via liblcms2 (CMS_FLAGS_GAMUTCHECK):
        // Neutral mid-gray is inside printer gamut; hyper-saturated ProPhoto RGB (65535, 0, 65535) is out of gamut.
        assert!(!prof.is_out_of_gamut(WorkingColorSpace::Srgb, Rgb16::new(32768, 32768, 32768)));
        assert!(prof.is_out_of_gamut(WorkingColorSpace::ProPhotoRgb, Rgb16::new(65535, 0, 65535)));

        // Batch buffer transform via liblcms2
        let mut print_buf = ImageBuffer16::new_solid(4, 4, Rgb16::WHITE).unwrap();
        prof.transform_buffer_in_place(
            WorkingColorSpace::Srgb,
            &mut print_buf,
            RenderingIntent::RelativeColorimetric,
            true,
        );
        assert_eq!(print_buf.get_pixel(0, 0).unwrap(), Rgb16::WHITE);

        // Softproof test on a 2x2 16-bit buffer
        let mut buf = ImageBuffer16::new_solid(2, 2, Rgb16::new(32768, 16000, 48000)).unwrap();
        prof.apply_softproof_to_display(
            WorkingColorSpace::Srgb,
            WorkingColorSpace::DisplayP3,
            &mut buf,
            RenderingIntent::Perceptual,
            true,
            SoftproofConfig {
                enabled: true,
                simulate_paper_white: true,
                viewing_light: ViewingLight::GalleryWarm3200K,
                gamut_warning: false,
                gamut_warning_color: Rgb8::new(255, 0, 200),
            },
        );
        let p = buf.get_pixel(0, 0).unwrap();
        assert!(p.r > 0 && p.b > 0);
    }

    #[test]
    fn lcms2_virtual_working_spaces_match_analytical_d50_matrices() {
        // Verify that our 4 virtual liblcms2 profiles (sRGB, AdobeRGB, ProPhoto, DisplayP3)
        // agree closely with our analytical D50 Bradford matrices across test colors.
        let test_colors = [
            Rgb16::new(65535, 65535, 65535),
            Rgb16::new(32768, 32768, 32768),
            Rgb16::new(50000, 20000, 15000),
            Rgb16::new(15000, 45000, 20000),
            Rgb16::new(12000, 18000, 52000),
        ];
        for src in WorkingColorSpace::ALL {
            for dst in WorkingColorSpace::ALL {
                let src_h = src.lcms_profile().expect("src profile");
                let dst_h = dst.lcms_profile().expect("dst profile");
                let xform = lcms2::TransformHandle::new_matrix_shaper_rgb16(src_h, dst_h)
                    .expect("transform");

                for &px in &test_colors {
                    let [lr, lg, lb] = xform.transform_pixel([px.r, px.g, px.b]);
                    let analytical = src.convert_pixel_to(px, dst);
                    assert!(
                        (i32::from(lr) - i32::from(analytical.r)).abs() <= 32
                            && (i32::from(lg) - i32::from(analytical.g)).abs() <= 32
                            && (i32::from(lb) - i32::from(analytical.b)).abs() <= 32,
                        "{src:?} -> {dst:?} for {px:?}: lcms2=({lr},{lg},{lb}) vs analytical={analytical:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn lcms2_gray_ramp_monotonicity_bpc_shadow_separation_and_rayon_bit_identity() {
        let icc_path =
            Path::new("/usr/share/color/icc/epson-scp900/Epson_SC-P900_700_EpsonPremiumGlossy.icc");
        if !icc_path.exists() {
            return;
        }
        let prof = IccProfile::from_file(icc_path).expect("valid Epson SC-P900 ICC profile");

        // 1. 256-step neutral gray ramp must be monotonically non-decreasing in R, G, B
        // and reach exact (65535, 65535, 65535) at step 255.
        let mut ramp_buf = ImageBuffer16::new_solid(256, 8, Rgb16::BLACK).unwrap();
        for y in 0..8 {
            for x in 0..256u32 {
                let v = (x as u16) * 257;
                ramp_buf.set_pixel(x, y, Rgb16::new(v, v, v));
            }
        }

        // Compare single-pixel transform vs parallel rayon buffer transform for bit-exact identity
        let expected_row: Vec<Rgb16> = (0..256u32)
            .map(|x| {
                let v = (x as u16) * 257;
                prof.transform_pixel(
                    WorkingColorSpace::Srgb,
                    Rgb16::new(v, v, v),
                    RenderingIntent::RelativeColorimetric,
                    true,
                )
            })
            .collect();

        prof.transform_buffer_in_place(
            WorkingColorSpace::Srgb,
            &mut ramp_buf,
            RenderingIntent::RelativeColorimetric,
            true,
        );

        for x in 0..256u32 {
            let actual = ramp_buf.get_pixel(x, 0).unwrap();
            assert_eq!(
                actual, expected_row[x as usize],
                "rayon buffer transform must be bit-identical to single-pixel transform at x={x}"
            );
            if x > 0 {
                let prev = expected_row[(x - 1) as usize];
                assert!(
                    actual.r >= prev.r && actual.g >= prev.g && actual.b >= prev.b,
                    "gray ramp non-monotonic at step {x}: prev={prev:?}, curr={actual:?}"
                );
            }
        }
        assert_eq!(expected_row[255], Rgb16::WHITE);

        // 2. Verify Black Point Compensation (BPC) lifts deep shadows in Relative Colorimetric
        // compared to BPC=false (where L* below media black gets clipped).
        let deep_shadow = Rgb16::new(1500, 1500, 1500);
        let with_bpc = prof.transform_pixel(
            WorkingColorSpace::Srgb,
            deep_shadow,
            RenderingIntent::RelativeColorimetric,
            true,
        );
        let without_bpc = prof.transform_pixel(
            WorkingColorSpace::Srgb,
            deep_shadow,
            RenderingIntent::RelativeColorimetric,
            false,
        );
        assert!(
            with_bpc.g > without_bpc.g,
            "Expected BPC=true ({with_bpc:?}) to preserve lighter shadow step than BPC=false ({without_bpc:?})"
        );

        // 3. Verify softproof gamut warning overlay replaces out-of-gamut pixels with warning color
        let mut proof_buf =
            ImageBuffer16::new_solid(2, 1, Rgb16::new(32768, 32768, 32768)).unwrap();
        proof_buf.set_pixel(1, 0, Rgb16::new(65535, 0, 65535)); // out-of-gamut in ProPhotoRGB
        let warn_color = Rgb8::new(255, 0, 200);
        prof.apply_softproof_to_display(
            WorkingColorSpace::ProPhotoRgb,
            WorkingColorSpace::Srgb,
            &mut proof_buf,
            RenderingIntent::RelativeColorimetric,
            true,
            SoftproofConfig {
                enabled: true,
                simulate_paper_white: true,
                viewing_light: ViewingLight::D50,
                gamut_warning: true,
                gamut_warning_color: warn_color,
            },
        );
        let in_gamut_px = proof_buf.get_pixel(0, 0).unwrap();
        let oog_px = proof_buf.get_pixel(1, 0).unwrap();
        assert_eq!(oog_px, warn_color.to_rgb16());
        assert_ne!(in_gamut_px, warn_color.to_rgb16());

        // 4. Verify all 4 Rendering Intents across multiple Epson SC-P900/P700 paper ICC profiles
        let icc_dir = Path::new("/usr/share/color/icc/epson-scp900");
        if let Ok(entries) = std::fs::read_dir(icc_dir) {
            let mut tested_profiles = 0usize;
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) != Some("icc") {
                    continue;
                }
                let paper_prof = IccProfile::from_file(&p).expect("load Epson paper ICC");
                for intent in RenderingIntent::ALL {
                    let w = paper_prof.transform_pixel(
                        WorkingColorSpace::Srgb,
                        Rgb16::WHITE,
                        intent,
                        true,
                    );
                    if intent != RenderingIntent::AbsoluteColorimetric {
                        assert_eq!(
                            w,
                            Rgb16::WHITE,
                            "Pure white must remain (65535,65535,65535) for {:?} in {}",
                            intent,
                            paper_prof.description
                        );
                    }
                    let mid = paper_prof.transform_pixel(
                        WorkingColorSpace::AdobeRgb1998,
                        Rgb16::new(32768, 32768, 32768),
                        intent,
                        true,
                    );
                    assert!(
                        mid.r > 10_000 && mid.r < 55_000,
                        "Mid-gray out of expected range for {:?} in {}: {:?}",
                        intent,
                        paper_prof.description,
                        mid
                    );
                }
                tested_profiles += 1;
            }
            assert!(
                tested_profiles >= 5,
                "Expected at least 5 Epson ICC profiles, found {tested_profiles}"
            );
        }
    }

    fn rgb_profile_bytes(
        white: (f64, f64),
        primaries: [(f64, f64); 3],
        gamma: f64,
    ) -> Option<Vec<u8>> {
        let xy = |(x, y): (f64, f64)| lcms2::CmsCIExyY { x, y, big_y: 1.0 };
        let [r, g, b] = primaries;
        let triple = lcms2::CmsCIExyYTriple {
            red: xy(r),
            green: xy(g),
            blue: xy(b),
        };
        lcms2::ProfileHandle::create_parametric_rgb(xy(white), triple, 1, &[gamma])?.to_bytes()
    }

    fn convert_embedded(bytes: &[u8], data: Vec<u16>) -> Option<(WorkingColorSpace, Vec<u16>)> {
        let prof = IccProfile::from_bytes(bytes, None).ok()?;
        let pixels = u32::try_from(data.len() / 3).ok()?;
        let mut buf = ImageBuffer16::from_raw(pixels, 1, data).ok()?;
        let ws = prof.convert_embedded_buffer_to_working_space(&mut buf)?;
        Some((ws, buf.data))
    }

    #[test]
    fn embedded_builtin_profiles_are_recognized_without_altering_samples() {
        let data = vec![
            0, 0, 0, 65_535, 65_535, 65_535, 12_345, 45_678, 34_567, 65_535, 0, 0,
        ];
        for space in WorkingColorSpace::ALL {
            let Some(bytes) = space.icc_profile_bytes() else {
                panic!("{space:?}: no ICC bytes");
            };
            let Some((ws, out)) = convert_embedded(&bytes, data.clone()) else {
                panic!("{space:?}: conversion failed");
            };
            assert_eq!(ws, space);
            assert_eq!(out, data, "{space:?}: samples must stay untouched");
        }
    }

    #[test]
    fn embedded_profiles_are_converted_by_colorants_and_curves_not_by_name() {
        // Linear-gamma profile with sRGB primaries: stays in sRGB, but linear 0.5 must be re-encoded (~0.735).
        let srgb_primaries = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
        let Some(linear) = rgb_profile_bytes((0.3127, 0.3290), srgb_primaries, 1.0) else {
            panic!("linear profile");
        };
        let Some((ws, out)) = convert_embedded(&linear, vec![32_768, 32_768, 32_768]) else {
            panic!("linear conversion failed");
        };
        assert_eq!(ws, WorkingColorSpace::Srgb);
        for v in out {
            assert!((47_900..=48_500).contains(&v), "linear 0.5 encoded as {v}");
        }

        // Adobe Wide Gamut RGB exceeds Adobe RGB and must land in ProPhoto RGB.
        let wide_primaries = [(0.7347, 0.2653), (0.1152, 0.8264), (0.1566, 0.0177)];
        let Some(wide) = rgb_profile_bytes((0.3457, 0.3585), wide_primaries, 2.2) else {
            panic!("wide-gamut profile");
        };
        let Some((ws, _)) = convert_embedded(&wide, vec![65_535, 0, 0]) else {
            panic!("wide-gamut conversion failed");
        };
        assert_eq!(ws, WorkingColorSpace::ProPhotoRgb);
    }
}
