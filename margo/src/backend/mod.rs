pub mod udev;
pub mod winit;

use smithay::backend::allocator::{Modifier, format::FormatSet};

/// Intel modifiers whose buffers carry a separate CCS (compression control) plane: Gen9 Y/Yf CCS, Gen12 RC/MC CCS(+CC),
/// DG2 and Meteor Lake 4-tiled RC/MC CCS(+CC) (drm_fourcc.h `I915_FORMAT_MOD_*`, vendor 0x01, codes 4-8 and 0xa-0xf).
/// Xe2's LNL/BMG CCS (0x10, 0x11) keep compression in the main surface (one plane) and are not listed.
pub(crate) fn is_intel_aux_ccs(modifier: Modifier) -> bool {
    let raw = u64::from(modifier);
    raw >> 56 == 0x01 && matches!(raw & 0x00ff_ffff_ffff_ffff, 4..=8 | 0xa..=0xf)
}

/// The formats margo offers clients over linux-dmabuf. Without `dmabuf_ccs`, Intel's aux-plane CCS modifiers are left
/// out: Mesa 26.3.1 sizes such buffers' plane 0 below stride x height (a 3x1440 MTL_RC_CCS_CC surface: stride 512,
/// plane 1 at 724992 < 512 * 1440), which linux-dmabuf rejects as out of bounds (smithay, wlroots and weston all check
/// it), so GTK's Vulkan renderer retried a lost surface at 100% CPU and its GL renderer died with protocol error 71.
/// Clients fall back to the plain (4-)tiled modifiers.
pub(crate) fn client_dmabuf_formats(formats: &FormatSet, allow_ccs: bool) -> FormatSet {
    formats
        .iter()
        .filter(|format| allow_ccs || !is_intel_aux_ccs(format.modifier))
        .copied()
        .collect::<FormatSet>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::allocator::{Format, Fourcc};

    fn set(modifiers: &[u64]) -> FormatSet {
        modifiers
            .iter()
            .map(|&m| Format {
                code: Fourcc::Abgr8888,
                modifier: Modifier::from(m),
            })
            .collect()
    }

    fn modifiers(formats: &FormatSet) -> Vec<u64> {
        formats.iter().map(|f| u64::from(f.modifier)).collect()
    }

    #[test]
    fn intel_aux_ccs_modifiers_are_left_out_by_default() {
        let linear = 0;
        let x_tiled = 0x0100_0000_0000_0001;
        let tile4 = 0x0100_0000_0000_0009;
        let mtl_rc_ccs_cc = 0x0100_0000_0000_000f; // what Mesa 26.3.1 picked for mshell's thin surfaces
        let gen12_rc_ccs = 0x0100_0000_0000_0006;
        let lnl_ccs = 0x0100_0000_0000_0010;
        let amd = 0x0200_0000_0000_0001;
        let all = set(&[
            linear,
            x_tiled,
            tile4,
            mtl_rc_ccs_cc,
            gen12_rc_ccs,
            lnl_ccs,
            amd,
        ]);
        assert_eq!(
            modifiers(&client_dmabuf_formats(&all, false)),
            vec![linear, x_tiled, tile4, lnl_ccs, amd]
        );
        assert_eq!(
            modifiers(&client_dmabuf_formats(&all, true)).len(),
            7,
            "dmabuf_ccs = 1 offers them all"
        );
    }

    #[test]
    fn invalid_modifier_is_not_ccs() {
        assert!(!is_intel_aux_ccs(Modifier::Invalid));
        assert!(is_intel_aux_ccs(Modifier::from(0x0100_0000_0000_000d)));
    }
}
