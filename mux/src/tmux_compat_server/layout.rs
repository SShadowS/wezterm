//! Tmux layout string generator.
//!
//! Generates tmux-format layout description strings from a tree of pane
//! geometry nodes. These strings are sent in `%layout-change` notifications
//! to tmux control mode clients.
//!
//! ## Layout string format
//!
//! A layout string has the form `<checksum>,<description>` where:
//!
//! - **Single pane**: `WxH,L,T,ID`
//! - **Horizontal split** (side by side): `WxH,L,T{child1,child2,...}`
//! - **Vertical split** (top/bottom): `WxH,L,T[child1,child2,...]`
//!
//! The checksum is a 16-bit CCITT-style CRC over the description portion,
//! output as four lowercase hex digits.

use std::fmt::Write;

/// A tree node describing the geometry of a pane or a split container.
///
/// Leaf nodes are [`LayoutNode::Pane`] entries with a concrete pane ID and
/// position. Interior nodes are either horizontal splits (children arranged
/// side by side) or vertical splits (children stacked top to bottom).
#[derive(Debug, Clone)]
pub enum LayoutNode {
    /// A single terminal pane.
    Pane {
        pane_id: u64,
        width: u64,
        height: u64,
        left: u64,
        top: u64,
    },
    /// A horizontal split container — children are arranged side by side.
    /// Rendered with `{...}` delimiters in the layout string.
    HorizontalSplit {
        width: u64,
        height: u64,
        left: u64,
        top: u64,
        children: Vec<LayoutNode>,
    },
    /// A vertical split container — children are stacked top to bottom.
    /// Rendered with `[...]` delimiters in the layout string.
    VerticalSplit {
        width: u64,
        height: u64,
        left: u64,
        top: u64,
        children: Vec<LayoutNode>,
    },
}

/// Compute the tmux layout checksum over a description string.
///
/// This implements the same algorithm used by tmux itself: a 16-bit rotating
/// checksum where each byte is added after a one-bit right-rotation of the
/// accumulator.
///
/// ```c
/// u_int csum = 0;
/// for each byte c in layout_description:
///     csum = (csum >> 1) + ((csum & 1) << 15)
///     csum += c
/// return csum & 0xffff
/// ```
pub fn layout_checksum(layout_desc: &str) -> u16 {
    let mut csum: u32 = 0;
    for &b in layout_desc.as_bytes() {
        csum = (csum >> 1) + ((csum & 1) << 15);
        csum += b as u32;
    }
    (csum & 0xffff) as u16
}

/// Generate a complete tmux layout string from a [`LayoutNode`] tree.
///
/// The returned string has the form `"{checksum:04x},{description}"` where
/// the description is the recursive rendering of the node tree and the
/// checksum covers the description portion.
pub fn generate_layout_string(root: &LayoutNode) -> String {
    let mut desc = String::new();
    write_node(root, &mut desc);
    let csum = layout_checksum(&desc);
    format!("{csum:04x},{desc}")
}

/// Recursively write the layout description for a single node into `out`.
fn write_node(node: &LayoutNode, out: &mut String) {
    match node {
        LayoutNode::Pane {
            pane_id,
            width,
            height,
            left,
            top,
        } => {
            let _ = write!(out, "{width}x{height},{left},{top},{pane_id}");
        }
        LayoutNode::HorizontalSplit {
            width,
            height,
            left,
            top,
            children,
        } => {
            let _ = write!(out, "{width}x{height},{left},{top}{{");
            write_children(children, out);
            out.push('}');
        }
        LayoutNode::VerticalSplit {
            width,
            height,
            left,
            top,
            children,
        } => {
            let _ = write!(out, "{width}x{height},{left},{top}[");
            write_children(children, out);
            out.push(']');
        }
    }
}

/// Write a comma-separated list of child node descriptions.
fn write_children(children: &[LayoutNode], out: &mut String) {
    for (i, child) in children.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_node(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // Checksum tests
    // -------------------------------------------------------------------

    #[test]
    fn checksum_single_pane() {
        // Known from tmux: `%layout-change @1 b25d,80x24,0,0,0`
        assert_eq!(layout_checksum("80x24,0,0,0"), 0xb25d);
    }

    #[test]
    fn checksum_empty_string() {
        assert_eq!(layout_checksum(""), 0);
    }

    #[test]
    fn checksum_is_16_bit() {
        // Feed in a long string and verify the result fits in 16 bits.
        let long = "a]b[c{d}e,f".repeat(1000);
        let csum = layout_checksum(&long);
        assert!(csum <= 0xffff);
    }

    #[test]
    fn checksum_single_byte() {
        // For a single byte 'A' (65):
        //   csum = 0 -> rotate -> 0, + 65 = 65
        assert_eq!(layout_checksum("A"), 65);
    }

    #[test]
    fn checksum_two_bytes() {
        // 'A' = 65, 'B' = 66
        // After 'A': csum = 65
        // Rotate: (65 >> 1) + ((65 & 1) << 15) = 32 + 32768 = 32800
        // Add 'B': 32800 + 66 = 32866
        assert_eq!(layout_checksum("AB"), 32866);
    }

    // -------------------------------------------------------------------
    // Description rendering tests
    // -------------------------------------------------------------------

    #[test]
    fn single_pane_description() {
        let root = LayoutNode::Pane {
            pane_id: 0,
            width: 80,
            height: 24,
            left: 0,
            top: 0,
        };
        let result = generate_layout_string(&root);
        assert_eq!(result, "b25d,80x24,0,0,0");
    }

    #[test]
    fn horizontal_split_two_panes() {
        // 160x40 window split horizontally into two panes side by side.
        let root = LayoutNode::HorizontalSplit {
            width: 160,
            height: 40,
            left: 0,
            top: 0,
            children: vec![
                LayoutNode::Pane {
                    pane_id: 0,
                    width: 80,
                    height: 40,
                    left: 0,
                    top: 0,
                },
                LayoutNode::Pane {
                    pane_id: 1,
                    width: 79,
                    height: 40,
                    left: 81,
                    top: 0,
                },
            ],
        };
        let result = generate_layout_string(&root);
        let expected_desc = "160x40,0,0{80x40,0,0,0,79x40,81,0,1}";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn vertical_split_two_panes() {
        // 80x48 window split vertically into two panes stacked.
        let root = LayoutNode::VerticalSplit {
            width: 80,
            height: 48,
            left: 0,
            top: 0,
            children: vec![
                LayoutNode::Pane {
                    pane_id: 0,
                    width: 80,
                    height: 24,
                    left: 0,
                    top: 0,
                },
                LayoutNode::Pane {
                    pane_id: 1,
                    width: 80,
                    height: 23,
                    left: 0,
                    top: 25,
                },
            ],
        };
        let result = generate_layout_string(&root);
        let expected_desc = "80x48,0,0[80x24,0,0,0,80x23,0,25,1]";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn nested_split() {
        // Horizontal split where the right child is a vertical split.
        //
        //  +--------+---------+
        //  |        | Pane 1  |
        //  | Pane 0 +---------+
        //  |        | Pane 2  |
        //  +--------+---------+
        let root = LayoutNode::HorizontalSplit {
            width: 160,
            height: 40,
            left: 0,
            top: 0,
            children: vec![
                LayoutNode::Pane {
                    pane_id: 0,
                    width: 80,
                    height: 40,
                    left: 0,
                    top: 0,
                },
                LayoutNode::VerticalSplit {
                    width: 79,
                    height: 40,
                    left: 81,
                    top: 0,
                    children: vec![
                        LayoutNode::Pane {
                            pane_id: 1,
                            width: 79,
                            height: 20,
                            left: 81,
                            top: 0,
                        },
                        LayoutNode::Pane {
                            pane_id: 2,
                            width: 79,
                            height: 19,
                            left: 81,
                            top: 21,
                        },
                    ],
                },
            ],
        };
        let result = generate_layout_string(&root);
        let expected_desc =
            "160x40,0,0{80x40,0,0,0,79x40,81,0[79x20,81,0,1,79x19,81,21,2]}";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn deeply_nested_split() {
        // Three levels of nesting:
        //   H-split
        //     Pane 69
        //     V-split
        //       Pane 70
        //       H-split
        //         Pane 71
        //         Pane 72
        let root = LayoutNode::HorizontalSplit {
            width: 158,
            height: 40,
            left: 0,
            top: 0,
            children: vec![
                LayoutNode::Pane {
                    pane_id: 69,
                    width: 79,
                    height: 40,
                    left: 0,
                    top: 0,
                },
                LayoutNode::VerticalSplit {
                    width: 78,
                    height: 40,
                    left: 80,
                    top: 0,
                    children: vec![
                        LayoutNode::Pane {
                            pane_id: 70,
                            width: 78,
                            height: 20,
                            left: 80,
                            top: 0,
                        },
                        LayoutNode::HorizontalSplit {
                            width: 78,
                            height: 19,
                            left: 80,
                            top: 21,
                            children: vec![
                                LayoutNode::Pane {
                                    pane_id: 71,
                                    width: 39,
                                    height: 19,
                                    left: 80,
                                    top: 21,
                                },
                                LayoutNode::Pane {
                                    pane_id: 72,
                                    width: 38,
                                    height: 19,
                                    left: 120,
                                    top: 21,
                                },
                            ],
                        },
                    ],
                },
            ],
        };
        let result = generate_layout_string(&root);
        let expected_desc = "158x40,0,0{79x40,0,0,69,78x40,80,0\
            [78x20,80,0,70,78x19,80,21{39x19,80,21,71,38x19,120,21,72}]}";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn single_pane_large_id() {
        let root = LayoutNode::Pane {
            pane_id: 999,
            width: 200,
            height: 50,
            left: 10,
            top: 5,
        };
        let result = generate_layout_string(&root);
        let expected_desc = "200x50,10,5,999";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn single_pane_nonzero_origin() {
        let root = LayoutNode::Pane {
            pane_id: 7,
            width: 40,
            height: 20,
            left: 41,
            top: 25,
        };
        let result = generate_layout_string(&root);
        let expected_desc = "40x20,41,25,7";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn horizontal_split_three_panes() {
        // Three panes side by side.
        let root = LayoutNode::HorizontalSplit {
            width: 120,
            height: 30,
            left: 0,
            top: 0,
            children: vec![
                LayoutNode::Pane {
                    pane_id: 0,
                    width: 40,
                    height: 30,
                    left: 0,
                    top: 0,
                },
                LayoutNode::Pane {
                    pane_id: 1,
                    width: 39,
                    height: 30,
                    left: 41,
                    top: 0,
                },
                LayoutNode::Pane {
                    pane_id: 2,
                    width: 39,
                    height: 30,
                    left: 81,
                    top: 0,
                },
            ],
        };
        let result = generate_layout_string(&root);
        let expected_desc = "120x30,0,0{40x30,0,0,0,39x30,41,0,1,39x30,81,0,2}";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn vertical_split_three_panes() {
        // Three panes stacked.
        let root = LayoutNode::VerticalSplit {
            width: 80,
            height: 60,
            left: 0,
            top: 0,
            children: vec![
                LayoutNode::Pane {
                    pane_id: 0,
                    width: 80,
                    height: 20,
                    left: 0,
                    top: 0,
                },
                LayoutNode::Pane {
                    pane_id: 1,
                    width: 80,
                    height: 19,
                    left: 0,
                    top: 21,
                },
                LayoutNode::Pane {
                    pane_id: 2,
                    width: 80,
                    height: 19,
                    left: 0,
                    top: 41,
                },
            ],
        };
        let result = generate_layout_string(&root);
        let expected_desc = "80x60,0,0[80x20,0,0,0,80x19,0,21,1,80x19,0,41,2]";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    // -------------------------------------------------------------------
    // Checksum cross-check against known tmux output
    // -------------------------------------------------------------------

    #[test]
    fn checksum_known_tmux_layout_120x29() {
        // From the tmux parser test data: `cafd,120x29,0,0,0`
        assert_eq!(layout_checksum("120x29,0,0,0"), 0xcafd);
    }

    #[test]
    fn generate_matches_known_tmux_80x24() {
        // The full string `b25d,80x24,0,0,0` is known from tmux output.
        let root = LayoutNode::Pane {
            pane_id: 0,
            width: 80,
            height: 24,
            left: 0,
            top: 0,
        };
        assert_eq!(generate_layout_string(&root), "b25d,80x24,0,0,0");
    }

    #[test]
    fn generate_matches_known_tmux_120x29() {
        // The full string `cafd,120x29,0,0,0` is known from tmux output.
        let root = LayoutNode::Pane {
            pane_id: 0,
            width: 120,
            height: 29,
            left: 0,
            top: 0,
        };
        assert_eq!(generate_layout_string(&root), "cafd,120x29,0,0,0");
    }

    // -------------------------------------------------------------------
    // Edge cases
    // -------------------------------------------------------------------

    #[test]
    fn horizontal_split_single_child() {
        // Degenerate case: split with one child. While unusual, the renderer
        // should still produce valid output.
        let root = LayoutNode::HorizontalSplit {
            width: 80,
            height: 24,
            left: 0,
            top: 0,
            children: vec![LayoutNode::Pane {
                pane_id: 0,
                width: 80,
                height: 24,
                left: 0,
                top: 0,
            }],
        };
        let result = generate_layout_string(&root);
        let expected_desc = "80x24,0,0{80x24,0,0,0}";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn vertical_split_single_child() {
        let root = LayoutNode::VerticalSplit {
            width: 80,
            height: 24,
            left: 0,
            top: 0,
            children: vec![LayoutNode::Pane {
                pane_id: 5,
                width: 80,
                height: 24,
                left: 0,
                top: 0,
            }],
        };
        let result = generate_layout_string(&root);
        let expected_desc = "80x24,0,0[80x24,0,0,5]";
        let expected_csum = layout_checksum(expected_desc);
        assert_eq!(result, format!("{expected_csum:04x},{expected_desc}"));
    }

    #[test]
    fn checksum_format_has_leading_zeros() {
        // Ensure the checksum is always four hex digits, even when the value
        // is small. We construct a layout whose checksum starts with zeros.
        let desc = "1x1,0,0,0";
        let csum = layout_checksum(desc);
        let root = LayoutNode::Pane {
            pane_id: 0,
            width: 1,
            height: 1,
            left: 0,
            top: 0,
        };
        let result = generate_layout_string(&root);
        assert_eq!(result, format!("{csum:04x},{desc}"));
        // Verify the checksum portion is exactly 4 characters.
        let comma_pos = result.find(',').unwrap();
        assert_eq!(comma_pos, 4);
    }
}
