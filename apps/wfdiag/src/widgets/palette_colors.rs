//! The shell's colour palette and its theme derivation.

#![deny(unsafe_code)]

use windows_reactor::*;

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Palette {
    pub(crate) panel: Color,
    pub(crate) card: Color,
    pub(crate) card_strong: Color,
    pub(crate) border: Color,
    pub(crate) dim: Color,
    pub(crate) active: Color,
    pub(crate) accent: Color,
    pub(crate) ok: Color,
    pub(crate) ok_bg: Color,
    pub(crate) warn: Color,
    pub(crate) warn_bg: Color,
    pub(crate) err: Color,
    pub(crate) err_bg: Color,
    pub(crate) text: Color,
    pub(crate) muted: Color,
}

impl Palette {
    pub(crate) fn for_theme(theme: WindowTheme) -> Self {
        if theme == WindowTheme::Light {
            Self {
                panel: Color::argb(140, 255, 255, 255),
                card: Color::argb(168, 255, 255, 255),
                card_strong: Color::rgb(255, 255, 255),
                border: Color::argb(23, 9, 20, 32),
                dim: Color::argb(34, 243, 247, 250),
                active: Color::argb(28, 15, 108, 189),
                accent: Color::rgb(15, 108, 189),
                ok: Color::rgb(28, 157, 91),
                ok_bg: Color::argb(42, 80, 205, 137),
                warn: Color::rgb(168, 127, 0),
                warn_bg: Color::argb(42, 241, 188, 0),
                err: Color::rgb(217, 33, 78),
                err_bg: Color::argb(40, 241, 65, 108),
                text: Color::rgb(26, 27, 27),
                muted: Color::rgb(95, 96, 96),
            }
        } else {
            Self {
                panel: Color::argb(153, 26, 27, 29),
                card: Color::argb(13, 255, 255, 255),
                card_strong: Color::rgb(43, 44, 46),
                border: Color::argb(20, 255, 255, 255),
                dim: Color::argb(112, 7, 9, 12),
                active: Color::argb(38, 77, 163, 232),
                accent: Color::rgb(77, 163, 232),
                ok: Color::rgb(80, 205, 137),
                ok_bg: Color::argb(44, 80, 205, 137),
                warn: Color::rgb(241, 188, 0),
                warn_bg: Color::argb(38, 241, 188, 0),
                err: Color::rgb(241, 65, 108),
                err_bg: Color::argb(48, 241, 65, 108),
                text: Color::rgb(255, 255, 255),
                muted: Color::rgb(207, 207, 207),
            }
        }
    }
}

pub(crate) fn palette_track() -> Color {
    Color::argb(26, 255, 255, 255)
}
