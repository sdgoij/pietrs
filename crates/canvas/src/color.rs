//! The 20 Piet colours and the two cyclic orders that define a command.

/// The six hues, in forward order: red, yellow, green, cyan, blue, magenta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Hue {
    Red = 0,
    Yellow = 1,
    Green = 2,
    Cyan = 3,
    Blue = 4,
    Magenta = 5,
}

impl Hue {
    /// Every hue in forward order.
    pub const ALL: [Hue; 6] = [
        Hue::Red,
        Hue::Yellow,
        Hue::Green,
        Hue::Cyan,
        Hue::Blue,
        Hue::Magenta,
    ];

    /// The hue's position in the cycle.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// Steps forward around the hue cycle to `next`, modulo 6.
    pub const fn forward_to(self, next: Hue) -> u8 {
        (next.index() + 6 - self.index()) % 6
    }
}

/// The three lightnesses, in forward ("darker") order: light, normal, dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Lightness {
    Light = 0,
    Normal = 1,
    Dark = 2,
}

impl Lightness {
    /// Every lightness in forward order.
    pub const ALL: [Lightness; 3] = [Lightness::Light, Lightness::Normal, Lightness::Dark];

    /// The lightness's position in the cycle.
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// Steps forward around the lightness cycle to `next`, modulo 3.
    pub const fn forward_to(self, next: Lightness) -> u8 {
        (next.index() + 3 - self.index()) % 3
    }
}

/// One of the 20 colours a Piet program may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    /// One of the 18 cycle colours.
    Chromatic { hue: Hue, lightness: Lightness },
    /// The free zone.
    White,
    /// The obstacle.
    Black,
}

impl Color {
    /// Every colour, chromatic first and then white and black.
    pub const ALL: [Color; 20] = [
        Color::Chromatic {
            hue: Hue::Red,
            lightness: Lightness::Light,
        },
        Color::Chromatic {
            hue: Hue::Red,
            lightness: Lightness::Normal,
        },
        Color::Chromatic {
            hue: Hue::Red,
            lightness: Lightness::Dark,
        },
        Color::Chromatic {
            hue: Hue::Yellow,
            lightness: Lightness::Light,
        },
        Color::Chromatic {
            hue: Hue::Yellow,
            lightness: Lightness::Normal,
        },
        Color::Chromatic {
            hue: Hue::Yellow,
            lightness: Lightness::Dark,
        },
        Color::Chromatic {
            hue: Hue::Green,
            lightness: Lightness::Light,
        },
        Color::Chromatic {
            hue: Hue::Green,
            lightness: Lightness::Normal,
        },
        Color::Chromatic {
            hue: Hue::Green,
            lightness: Lightness::Dark,
        },
        Color::Chromatic {
            hue: Hue::Cyan,
            lightness: Lightness::Light,
        },
        Color::Chromatic {
            hue: Hue::Cyan,
            lightness: Lightness::Normal,
        },
        Color::Chromatic {
            hue: Hue::Cyan,
            lightness: Lightness::Dark,
        },
        Color::Chromatic {
            hue: Hue::Blue,
            lightness: Lightness::Light,
        },
        Color::Chromatic {
            hue: Hue::Blue,
            lightness: Lightness::Normal,
        },
        Color::Chromatic {
            hue: Hue::Blue,
            lightness: Lightness::Dark,
        },
        Color::Chromatic {
            hue: Hue::Magenta,
            lightness: Lightness::Light,
        },
        Color::Chromatic {
            hue: Hue::Magenta,
            lightness: Lightness::Normal,
        },
        Color::Chromatic {
            hue: Hue::Magenta,
            lightness: Lightness::Dark,
        },
        Color::White,
        Color::Black,
    ];

    /// A chromatic colour.
    pub const fn chromatic(hue: Hue, lightness: Lightness) -> Color {
        Color::Chromatic { hue, lightness }
    }

    /// Whether this is one of the 18 cycle colours.
    pub const fn is_chromatic(self) -> bool {
        matches!(self, Color::Chromatic { .. })
    }

    /// The hue, for a chromatic colour.
    pub const fn hue(self) -> Option<Hue> {
        match self {
            Color::Chromatic { hue, .. } => Some(hue),
            Color::White | Color::Black => None,
        }
    }

    /// The lightness, for a chromatic colour.
    pub const fn lightness(self) -> Option<Lightness> {
        match self {
            Color::Chromatic { lightness, .. } => Some(lightness),
            Color::White | Color::Black => None,
        }
    }

    /// The colour's RGB value.
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            Color::White => [255, 255, 255],
            Color::Black => [0, 0, 0],
            Color::Chromatic { hue, lightness } => match (hue, lightness) {
                (Hue::Red, Lightness::Light) => [255, 192, 192],
                (Hue::Yellow, Lightness::Light) => [255, 255, 192],
                (Hue::Green, Lightness::Light) => [192, 255, 192],
                (Hue::Cyan, Lightness::Light) => [192, 255, 255],
                (Hue::Blue, Lightness::Light) => [192, 192, 255],
                (Hue::Magenta, Lightness::Light) => [255, 192, 255],
                (Hue::Red, Lightness::Normal) => [255, 0, 0],
                (Hue::Yellow, Lightness::Normal) => [255, 255, 0],
                (Hue::Green, Lightness::Normal) => [0, 255, 0],
                (Hue::Cyan, Lightness::Normal) => [0, 255, 255],
                (Hue::Blue, Lightness::Normal) => [0, 0, 255],
                (Hue::Magenta, Lightness::Normal) => [255, 0, 255],
                (Hue::Red, Lightness::Dark) => [192, 0, 0],
                (Hue::Yellow, Lightness::Dark) => [192, 192, 0],
                (Hue::Green, Lightness::Dark) => [0, 192, 0],
                (Hue::Cyan, Lightness::Dark) => [0, 192, 192],
                (Hue::Blue, Lightness::Dark) => [0, 0, 192],
                (Hue::Magenta, Lightness::Dark) => [192, 0, 192],
            },
        }
    }

    /// The palette colour with exactly this RGB value.
    pub fn from_rgb(rgb: [u8; 3]) -> Option<Color> {
        Self::ALL.iter().copied().find(|color| color.rgb() == rgb)
    }

    /// The palette colour for an RGB value: an exact match, or white for a
    /// non-standard colour.
    pub fn quantize(rgb: [u8; 3]) -> Color {
        Self::from_rgb(rgb).unwrap_or(Color::White)
    }

    /// The forward hue and lightness deltas between two chromatic colours.
    pub const fn deltas(self, next: Color) -> Option<(u8, u8)> {
        match (self, next) {
            (
                Color::Chromatic {
                    hue: from_hue,
                    lightness: from_lightness,
                },
                Color::Chromatic {
                    hue: to_hue,
                    lightness: to_lightness,
                },
            ) => Some((
                from_hue.forward_to(to_hue),
                from_lightness.forward_to(to_lightness),
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn the_palette_has_twenty_distinct_colours() {
        assert_eq!(Color::ALL.len(), 20);
        let mut seen = HashSet::new();
        for color in Color::ALL {
            assert!(seen.insert(color.rgb()), "duplicate RGB for {color:?}");
        }
        assert_eq!(Color::White.rgb(), [255, 255, 255]);
        assert_eq!(Color::Black.rgb(), [0, 0, 0]);
    }

    #[test]
    fn every_colour_round_trips_through_rgb() {
        for color in Color::ALL {
            assert_eq!(Color::from_rgb(color.rgb()), Some(color));
        }
    }

    #[test]
    fn the_hue_cycle_is_forward_by_one() {
        for (index, from) in Hue::ALL.iter().enumerate() {
            let next = Hue::ALL[(index + 1) % Hue::ALL.len()];
            assert_eq!(from.forward_to(next), 1);
            assert_eq!(next.forward_to(*from), 5);
        }
    }

    #[test]
    fn the_lightness_cycle_is_forward_by_one() {
        for (index, from) in Lightness::ALL.iter().enumerate() {
            let next = Lightness::ALL[(index + 1) % Lightness::ALL.len()];
            assert_eq!(from.forward_to(next), 1);
            assert_eq!(next.forward_to(*from), 2);
        }
    }

    #[test]
    fn deltas_are_only_defined_between_chromatic_colours() {
        let red_light = Color::chromatic(Hue::Red, Lightness::Light);
        let green_dark = Color::chromatic(Hue::Green, Lightness::Dark);
        assert_eq!(red_light.deltas(green_dark), Some((2, 2)));
        assert_eq!(red_light.deltas(Color::White), None);
        assert_eq!(Color::Black.deltas(green_dark), None);
    }

    #[test]
    fn a_non_standard_colour_quantizes_to_white() {
        assert_eq!(Color::from_rgb([1, 2, 3]), None);
        assert_eq!(Color::quantize([1, 2, 3]), Color::White);
        assert_eq!(
            Color::quantize([255, 0, 0]),
            Color::chromatic(Hue::Red, Lightness::Normal)
        );
    }
}
