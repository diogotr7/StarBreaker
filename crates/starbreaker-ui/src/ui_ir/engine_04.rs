#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use sha2::{Digest, Sha256};
#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};
#[allow(unused_imports)]
use crate::bb_bindings::BindingResolver;
#[allow(unused_imports)]
use crate::bb_layout;
#[allow(unused_imports)]
use crate::bb_layout::{LayoutResult, Rect};
#[allow(unused_imports)]
use crate::bb_scene::{BbNode, BbNodeId, BbNodeType, BbScene, BbValue};
#[allow(unused_imports)]
use crate::defaults::DefaultValueRegistry;
#[allow(unused_imports)]
use crate::pipeline::CanvasFetcher;

// Consolidated engine chunk 04 (formerly: part_14.part, part_15.part, pagein_alpha_tests.part, part_16.part, part_17.part, part_18.part).

#[cfg(test)]
mod tests_d {
    #![allow(unused_imports, dead_code)]

    use super::*;

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }


    #[test]
    fn compile_ir_samples_animation_alpha_at_requested_percent() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAnimationSample",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "fingerprint_like",
                        "isActive": true,
                        "alpha": 1.0,
                        "text": "SCAN",
                        "animation": {
                            "animationTimeline": {
                                "keyframes": [
                                    {
                                        "percent": 0.0,
                                        "modifiers": [{"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "Alpha", "value": 0.0}}]
                                    },
                                    {
                                        "percent": 1.0,
                                        "modifiers": [{"modifier": {"_Type_": "BuildingBlocks_FieldModifierNumber", "field": "Alpha", "value": 1.0}}]
                                    }
                                ]
                            }
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene_with_animation_sample(
            &scene,
            None,
            "guid-animation-sample",
            Some("BuildingBlocks_Canvas.TestAnimationSample"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            Some(50.0),
            100,
            1.0,
            None,
            false,
            false,
        );

        let node = ir.nodes.iter().find(|node| node.name == "fingerprint_like").expect("sample node");
        assert!((node.alpha - 0.5).abs() < f32::EPSILON);
    }

    /// With a [`DrawTextMeasure`] supplied, the pre-layout annotation pass
    /// writes `_DrawTextWidthPx_`/`_DrawTextHeightPx_` from the DRAW-side
    /// glyph metrics and the intrinsic layout hugs them; without it the
    /// TTF estimate applies (the control proves the assertion discriminates).
    #[test]
    fn compile_ir_uses_draw_text_measure_for_intrinsic_boxes() {
        struct StubMeasure;
        impl DrawTextMeasure for StubMeasure {
            fn measure_px(
                &self,
                _font_symbol: Option<&str>,
                _label_style: Option<&str>,
                _text: &str,
                _font_px: f32,
                _letter_spacing_px: f32,
            ) -> Option<(f32, f32)> {
                Some((160.4, 30.0))
            }
        }

        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestDrawMeasure",
            "_RecordValue_": {
                "size": {"x": 400, "y": 100},
                "coordinateMethod": "useRaw",
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_DisplayWidget",
                     "name": "row", "isActive": true,
                     "layoutPolicy": {"_Type_": "BuildingBlocks_FlexContainer",
                                      "direction": "Row", "axisJustification": "Start"},
                     "sizing": {"width": {"value": 1.0, "behavior": "Percent"},
                                "height": {"value": 1.0, "behavior": "Percent"}}},
                    {"_Pointer_": "ptr:2", "_Type_": "BuildingBlocks_WidgetTextField",
                     "name": "title", "parent": "_PointsTo_:ptr:1", "isActive": true,
                     "_ResolvedText_": "OUTPUT",
                     "text": "OUTPUT",
                     "fontSize": {"value": 20.0, "behavior": "Fixed"},
                     "sizing": {"width": {"value": 0.0, "behavior": "Auto"},
                                "height": {"value": 1.0, "behavior": "Percent"}}}
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let compile = |measure: Option<&dyn DrawTextMeasure>| {
            compile_ui_ir_from_scene_with_animation_sample(
                &scene,
                None,
                "guid-draw-measure",
                Some("BuildingBlocks_Canvas.TestDrawMeasure"),
                (400, 100),
                &defaults(),
                None,
                None,
                &[],
                Vec::new(),
                Vec::new(),
                None,
                100,
                1.0,
                measure,
                false,
                false,
            )
        };

        let with_measure = compile(Some(&StubMeasure));
        let title = with_measure
            .nodes
            .iter()
            .find(|node| node.name == "title")
            .expect("title node");
        assert!(
            (title.computed_rect.w - 160.4).abs() < 0.6,
            "the Auto title box hugs the draw-measured width, got {}",
            title.computed_rect.w
        );

        // Control: the same compile WITHOUT the measurer uses the TTF
        // estimate — if this also landed at 160.4 the assertion above would
        // prove nothing.
        let without_measure = compile(None);
        let control = without_measure
            .nodes
            .iter()
            .find(|node| node.name == "title")
            .expect("title node");
        assert!(
            (control.computed_rect.w - 160.4).abs() > 5.0,
            "the TTF-estimate control must differ from the stub width, got {}",
            control.computed_rect.w
        );
    }

    #[test]
    fn compile_ir_emits_border_and_stroke_metadata() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestBorderStroke",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "bordered",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        },
                        "border": {
                            "top": {"width": 2.0, "color": {"r": 255.0, "g": 0.0, "b": 0.0, "a": 255.0}},
                            "right": {"width": 3.0},
                            "bottom": {"width": 4.0},
                            "left": {"width": 5.0}
                        },
                        "StrokeColor": {"r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0},
                        "StrokeColorToken": "Accent4",
                        "strokeExtent": 1.5,
                        "BorderColorToken": "Accent1",
                        "BorderColorRightToken": "Accent2"
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-border-stroke",
            Some("BuildingBlocks_Canvas.TestBorderStroke"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "bordered").expect("bordered node");
        let border = node.border.as_ref().expect("border metadata");
        assert_eq!(border.top.width, 2.0);
        assert_eq!(border.top.colour, Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(border.top.colour_token.as_deref(), Some("Accent1"));
        assert_eq!(border.right.colour_token.as_deref(), Some("Accent2"));
        assert_eq!(node.stroke_colour, Some([0.25, 0.5, 0.75, 1.0]));
        assert_eq!(node.stroke_colour_token.as_deref(), Some("Accent4"));
        assert_eq!(node.stroke_extent, Some(1.5));
    }

    #[test]
    fn compile_ir_reads_svg_fill_stroke_extent() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestSvgFillStrokeExtent",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "separator",
                        "isActive": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 8.0}
                        },
                        "svgFill": {
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "strokeExtent": 1.0
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-svg-fill-stroke-extent",
            Some("BuildingBlocks_Canvas.TestSvgFillStrokeExtent"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "separator").expect("separator node");
        assert_eq!(node.stroke_extent, Some(1.0));
    }

    #[test]
    fn compile_ir_prefers_svg_fill_overlay_token_and_alpha() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestSvgFillOverlayTint",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "overlay",
                        "isActive": true,
                        "alpha": 1.0,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 8.0}
                        },
                        "svgFill": {
                            "svgPath": "UI/Textures/Vector/Drake/Drake_lowerline.svg",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Accent2",
                                "alpha": 0.2
                            }
                        },
                        "background": {
                            "enable": false,
                            "color": {
                                "_Type_": "BuildingBlocks_ColorStyle",
                                "color": "Base",
                                "alpha": 1.0
                            }
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-svg-fill-overlay-token",
            Some("BuildingBlocks_Canvas.TestSvgFillOverlayTint"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "overlay").expect("overlay node");
        assert_eq!(node.icon_tint_colour_token.as_deref(), Some("Accent2"));
        assert!((node.alpha - 0.2).abs() < 0.001, "expected overlay alpha 0.2, got {}", node.alpha);
    }

    #[test]
    fn compile_ir_emits_svg_flip_flags_in_asset_layout() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestSvgFlipFlags",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "arrow_right",
                        "isActive": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        },
                        "svgFill": {
                            "svgPath": "UI/Textures/Vector/H_HUDScreens/Ships/DRAK/drake_holo_hud_pixel_arrow.svg",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "flipHorizontal": true,
                            "flipVertical": false,
                            "scalingBehavior": "Contain",
                            "containPositionX": 0.5,
                            "containPositionY": 0.5
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-svg-flip-flags",
            Some("BuildingBlocks_Canvas.TestSvgFlipFlags"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "arrow_right").expect("arrow_right node");
        let layout = node.asset_layout.as_ref().expect("asset layout");
        assert_eq!(layout.flip_horizontal, Some(true));
        assert_eq!(layout.flip_vertical, None);
    }

    /// The authored svgFill strokeExtent is preserved regardless of the slot's
    /// fixed height — the visible strip THICKNESS is the widget-standard's
    /// Min/MaxSize clamp (`separator_strip`), never a magic height gate. (The
    /// former fixed-16px "procedural strip" special case suppressed the stroke
    /// extent so the whole slot filled; the Carrack lift-call bottom bar
    /// proved the reference draws the standard's clamped strip instead.)
    #[test]
    fn compile_ir_keeps_stroke_extent_for_fixed_height_separator_slots() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestAuthoredSeparatorStrip",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "separator",
                        "isActive": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 16.0}
                        },
                        "svgFill": {
                            "svgPath": "",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "enableNineSliceRect": true,
                            "strokeExtent": 1.0
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-authored-separator-strip",
            Some("BuildingBlocks_Canvas.TestAuthoredSeparatorStrip"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "separator").expect("separator node");
        assert_eq!(node.computed_rect.h, 16.0);
        assert_eq!(node.stroke_extent, Some(1.0));
    }

    /// The widget-standard brand entry's Min/MaxSize clamp + inner
    /// Anchor/Pivot land on the IR node as `separator_strip`, so the draw
    /// renders the clamped strip inside the authored slot box (synthetic
    /// values — real standards author e.g. uilo_a Primary 6/6 @0.5/0.5).
    #[test]
    fn compile_ir_separator_standard_strip_clamp_reaches_ir() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestSeparatorStripClamp",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "primary_separator",
                        "isActive": true,
                        "alpha": 1.0,
                        "direction": "Horizontal",
                        "style": "Primary",
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 16.0}
                        },
                        "svgFill": {
                            "svgPath": "",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "enableNineSliceRect": true,
                            "strokeExtent": 1.0
                        }
                    }
                ],
                "operations": []
            }
        });
        let standard_separator = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.HorizontalSeparatorPrimaryWidgetStandard",
            "_RecordValue_": {
                "brandStyles": [
                    {
                        "brandIdentifier": "file://./../../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_bioc.json",
                        "entries": [
                            {
                                "name": "Root",
                                "modifiers": [
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierBoolean",
                                        "field": "EnableMinHeight",
                                        "value": true
                                    },
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierBoolean",
                                        "field": "EnableMaxHeight",
                                        "value": true
                                    },
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                                        "field": "MinSizeY",
                                        "value": 5.0
                                    },
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                                        "field": "MaxSizeY",
                                        "value": 5.0
                                    },
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                                        "field": "PivotY",
                                        "value": 0.5
                                    },
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierNumber",
                                        "field": "AnchorY",
                                        "value": 0.5
                                    },
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierColor",
                                        "field": "BackgroundColor",
                                        "color": {
                                            "_Type_": "BuildingBlocks_ColorStyle",
                                            "color": "Base",
                                            "alpha": 1.0
                                        }
                                    }
                                ]
                            }
                        ]
                    }
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(
                "horizontal-primary".to_string(),
                standard_separator,
            )]),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-separator-strip-clamp",
            Some("BuildingBlocks_Canvas.TestSeparatorStripClamp"),
            (100, 100),
            &defaults(),
            Some("canvas:s_bioc".to_owned()),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|node| node.name == "primary_separator")
            .expect("primary separator node");
        let strip = node.separator_strip.as_ref().expect("separator strip from the standard");
        assert_eq!(strip.min_h, Some(5.0));
        assert_eq!(strip.max_h, Some(5.0));
        assert_eq!(strip.anchor_y, Some(0.5));
        assert_eq!(strip.pivot_y, Some(0.5));
        assert_eq!(strip.min_w, None);
        assert_eq!(node.stroke_colour_token.as_deref(), Some("Base"));
    }

    #[test]
    fn compile_ir_applies_horizontal_separator_source_style_colour() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestHorizontalSeparatorStyle",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "secondary_separator",
                        "isActive": true,
                        "alpha": 0.5,
                        "direction": "Horizontal",
                        "style": "Secondary",
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 8.0}
                        },
                        "svgFill": {
                            "svgPath": "",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "enableNineSliceRect": true,
                            "strokeExtent": 1.0
                        }
                    }
                ],
                "operations": []
            }
        });
        let standard_separator = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.HorizontalSeparatorSecondaryWidgetStandard",
            "_RecordValue_": {
                "brandStyles": [
                    {
                        "brandIdentifier": "file://./../../../../../../../../libs/foundry/records/ui/buildingblocks/styles/s_bioc.json",
                        "entries": [
                            {
                                "name": "Root",
                                "modifiers": [
                                    {
                                        "_Type_": "BuildingBlocks_FieldModifierColor",
                                        "field": "FillColor",
                                        "color": {
                                            "_Type_": "BuildingBlocks_ColorStyle",
                                            "color": "Accent1",
                                            "alpha": 0.3
                                        }
                                    }
                                ]
                            }
                        ]
                    }
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([(
                "horizontal-secondary".to_string(),
                standard_separator,
            )]),
        };

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            Some(&fetcher),
            "guid-horizontal-separator-style",
            Some("BuildingBlocks_Canvas.TestHorizontalSeparatorStyle"),
            (100, 100),
            &defaults(),
            Some("canvas:s_bioc".to_owned()),
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|node| node.name == "secondary_separator")
            .expect("secondary separator node");
        assert_eq!(node.stroke_colour_token.as_deref(), Some("Accent1"));
        assert_eq!(node.colour_blend_mode, Some(UiIrColourBlendMode::Additive));
        assert!((node.alpha - 0.15).abs() < 0.001, "expected styled alpha 0.15, got {}", node.alpha);
    }

    /// A Vertical Tertiary separator on an MFD frame (`design_text_scale > 1`)
    /// paints the standard's DEFAULT divider SVG — the dotted glyph the engine
    /// falls back to because DRAK ships no separator vector of its own. On a
    /// non-MFD (physical) frame it resolves NOTHING, so the medical bed /
    /// end-of-bed keep their byte-identical no-separator render.
    #[test]
    fn compile_ir_mfd_separator_paints_default_divider_but_skips_physical_screens() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestMfdSeparator",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {"_Pointer_": "ptr:1", "_Type_": "BuildingBlocks_WidgetSeparator",
                     "name": "title_separator", "isActive": true,
                     "direction": "Vertical", "style": "Tertiary",
                     "sizing": {"width": {"behavior": "Fixed", "value": 30.0},
                                "height": {"behavior": "Fixed", "value": 60.0}}}
                ],
                "operations": []
            }
        });
        let standard = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.VerticalSeparatorTertiaryWidgetStandard",
            "_RecordValue_": {
                "scene": [
                    {"_Type_": "BuildingBlocks_WidgetCustomShape", "name": "ComponentRoot",
                     "entries": [{"name": "Default", "modifiers": [
                        {"_Type_": "BuildingBlocks_FieldModifierString", "field": "SvgPath",
                         "value": "UI/Textures/Vector/MFD/PU_MFD_Generic_V_Divider.svg"}]}]}
                ],
                "brandStyles": [
                    {"brandIdentifier": "file://x/s_drak_env.json",
                     "entries": [{"name": "Root", "modifiers": [
                        {"_Type_": "BuildingBlocks_FieldModifierString", "field": "SvgPath",
                         "value": "UI/Textures/Vector/ModularKitStyles/DRAK_S42/absent.svg"},
                        {"_Type_": "BuildingBlocks_FieldModifierColor", "field": "FillColor",
                         "color": {"_Type_": "BuildingBlocks_ColorStyle", "color": "Base", "alpha": 1.0}}]}]}
                ]
            }
        });
        let fetcher = TestCanvasFetcher {
            by_guid: std::collections::HashMap::new(),
            by_path: std::collections::HashMap::from([("vertical-tertiary".to_string(), standard)]),
        };
        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let compile = |scale: f32| compile_ui_ir_from_scene_with_animation_sample(
            &scene, Some(&fetcher), "guid-mfd-sep",
            Some("BuildingBlocks_Canvas.TestMfdSeparator"), (100, 100), &defaults(),
            Some("manufacturer:drak".to_owned()), None, &[],
            Vec::new(), Vec::new(), None, 100, scale, None, false, false,
        );

        let mfd = compile(1.667);
        let sep = mfd.nodes.iter().find(|n| n.name == "title_separator").expect("sep");
        assert_eq!(
            sep.asset_ref.as_deref(),
            Some("UI/Textures/Vector/MFD/PU_MFD_Generic_V_Divider.svg"),
            "MFD separator paints the default divider SVG (the missing DRAK override is skipped)"
        );

        let physical = compile(1.0);
        let sep = physical.nodes.iter().find(|n| n.name == "title_separator").expect("sep");
        assert_eq!(
            sep.asset_ref, None,
            "physical-screen separator stays empty so medical platinum is byte-identical"
        );
    }
}

#[cfg(test)]
mod tests_e {
    #![allow(unused_imports, dead_code)]

    use super::*;

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }


    #[test]
    fn compile_ir_emits_effective_inherited_alpha() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestInheritedAlpha",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "parent",
                        "isActive": true,
                        "alpha": 0.5,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        },
                        "children": ["ptr:2"]
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "child",
                        "isActive": true,
                        "parent": "_PointsTo_:ptr:1",
                        "alpha": 0.62,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 8.0}
                        },
                        "svgFill": {
                            "svgPath": "",
                            "renderShape": true,
                            "enableColorOverlay": true,
                            "strokeExtent": 1.0
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-inherited-alpha",
            Some("BuildingBlocks_Canvas.TestInheritedAlpha"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let child = ir.nodes.iter().find(|node| node.name == "child").expect("child node");
        assert!((child.alpha - 0.31).abs() < 0.001, "expected inherited alpha 0.31, got {}", child.alpha);
    }

    #[test]
    fn compile_ir_emits_segmented_fill_metadata() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestSegmentedFill",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "segmented",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        },
                        "background": {
                            "enable": false,
                            "color": null
                        },
                        "segmentedFill": {
                            "enable": true,
                            "angle": 22.5
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-segmented",
            Some("BuildingBlocks_Canvas.TestSegmentedFill"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|node| node.name == "segmented").expect("segmented node");
        let segmented = node.segmented_fill.as_ref().expect("segmented fill metadata");
        assert!(segmented.enabled);
        assert_eq!(segmented.angle, 22.5);
        assert_eq!(segmented.segment_size, 64.0);
        assert_eq!(segmented.segment_spacing_size, 64.0);
    }

    #[test]
    fn compile_ir_emits_schema_and_nodes() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "isActive": true,
                        "position": {"x": 5.0, "y": 7.0},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        },
                        "text": "HELLO"
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-1",
            Some("BuildingBlocks_Canvas.Test"),
            (200, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            90,
        );

        assert_eq!(ir.schema_version, UI_IR_SCHEMA_VERSION);
        assert_eq!(ir.canvas_guid, "guid-1");
        assert_eq!(ir.target_width, 200);
        assert_eq!(ir.nodes.len(), 1);
        assert_eq!(ir.nodes[0].name, "label");
        assert_eq!(ir.nodes[0].node_type, "widget_text_field");
        assert_eq!(ir.nodes[0].text_payload, Some(UiIrTextPayload::Resolved { text: "HELLO".into() }));
    }

    #[test]
    fn compile_ir_is_deterministic_for_same_input() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test2",
            "_RecordValue_": {
                "size": {"x": 50, "y": 50},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "root",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Percent", "value": 1.0},
                            "height": {"behavior": "Percent", "value": 1.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir1 = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-2",
            None,
            (128, 128),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );
        let ir2 = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-2",
            None,
            (128, 128),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let s1 = serde_json::to_string(&ir1).expect("serialize ir1");
        let s2 = serde_json::to_string(&ir2).expect("serialize ir2");
        assert_eq!(s1, s2);

        let h1 = stable_hash_ui_ir(&ir1).expect("hash ir1");
        let h2 = stable_hash_ui_ir(&ir2).expect("hash ir2");
        assert_eq!(h1, h2);
    }

    #[test]
    fn validate_ir_rejects_invalid_document() {
        let invalid = UiIrDocument {
            schema_version: 999,
            canvas_guid: String::new(),
            canvas_name: None,
            target_width: 0,
            target_height: 0,
            selected_style_source: None,
            selected_swf_source: None,
            renderer_hint: UiRendererHint::Bb,
            confidence: 101,
            warnings: Vec::new(),
            unresolved_references: Vec::new(),
            resolved_asset_refs: Vec::new(),
            missing_asset_refs: Vec::new(),
            nodes: vec![UiIrNode {
                id: 1,
                parent_id: None,
                children: Vec::new(),
                node_type: String::new(),
                name: String::new(),
                is_active: true,
                layer: 0,
                alpha: 1.0,
                anchor: [0.0, 0.0],
                pivot: [0.0, 0.0],
                rotation_deg: None,
                authored_position: [0.0, 0.0],
                authored_size: [
                    UiIrValue::Fixed { value: 1.0 },
                    UiIrValue::Fixed { value: 1.0 },
                ],
                padding: [0.0, 0.0, 0.0, 0.0],
                margin: [0.0, 0.0, 0.0, 0.0],
                overflow_mode: None,
                clip_rect: None,
                computed_rect: UiIrRect {
                    x: 0.0,
                    y: 0.0,
                    w: -1.0,
                    h: -1.0,
                },
                background_fill_colour: None,
                corner_radius: None,
                corner_radii: None,
                corner_chamfers: None,
                background_fill_alpha: None,
                background_fill_colour_token: None,
                circle_fill_colour_token: None,
                style_provenance: None,
                segmented_fill: None,
            polygon: None,
                border: None,
                stroke_colour: None,
                stroke_colour_token: None,
                stroke_extent: None,
                separator_strip: None,
                colour_blend_mode: None,
                icon_tint_colour: None,
                icon_tint_colour_token: None,
                icon_preset: None,
                text_payload: None,
                secondary_text_payload: None,
                secondary_text_style: None,
                meter_progress: None,
                text_style: None,
                asset_ref: None,
                primitive_material: None,
                primitive_uv_start: None,
                primitive_uv_size: None,
                asset_layout: None,
                custom_shape: None,
                style_tag_uuids: Vec::new(),
                resolved_style_tags: Vec::new(),
                is_flash_renderer: false,
                auto_font_size: false,
            colour_overlay_enabled: false,
            }],
        };

        let result = validate_ui_ir_document(&invalid);
        assert!(result.is_err());
    }

    #[test]
    fn compile_ir_warns_when_separator_lacks_strict_style_semantics() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.SeparatorStyleGap",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "separator_gap",
                        "isActive": true,
                        "size": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 8.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-separator-gap",
            Some("BuildingBlocks_Canvas.SeparatorStyleGap"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        assert!(
            ir.warnings
                .iter()
                .any(|warning| warning.contains("strict-renderer style semantic gap")),
            "expected strict-renderer semantic warning, got: {:?}",
            ir.warnings
        );
        assert!(
            ir.warnings
                .iter()
                .any(|warning| warning.contains("separator missing stroke/background colour semantics")),
            "expected separator semantic detail warning, got: {:?}",
            ir.warnings
        );
    }

    #[test]
    fn compile_ir_populates_anchor_pivot_alpha_and_style_tags() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.Test3",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "alpha": 0.5,
                        "anchor": {"x": 0.25, "y": 0.75},
                        "pivot": {"x": 0.5, "y": 0.5},
                        "styleTags": [{"_RecordId_": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"}],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-3",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );
        let node = &ir.nodes[0];
        assert_eq!(node.alpha, 0.5);
        assert_eq!(node.anchor, [0.25, 0.75]);
        assert_eq!(node.pivot, [0.5, 0.5]);
        assert_eq!(node.style_tag_uuids.len(), 1);
    }

    #[test]
    fn compile_ir_adds_primary_state_tag_alongside_authored_style_tags() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.PrimaryStateStyleTags",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "styleTags": [{"_RecordId_": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"}],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": [
                    {
                        "_Type_": "BuildingBlocks_BindingsStringField",
                        "widget": "_PointsTo_:ptr:1",
                        "field": "PrimaryStateTag",
                        "input": "_PointsTo_:ptr:2"
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_BindingsStringComponentParameter",
                        "name": "primary state",
                        "parameter": "ParamInput0",
                        "defaultValue": "Tag.1477f18d-9b3e-4e5c-8047-dc60ba606ddb"
                    }
                ]
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-primary-state-style-tags",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        assert_eq!(node.style_tag_uuids.len(), 2);
        assert!(node
            .style_tag_uuids
            .iter()
            .any(|uuid| uuid == "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
        assert!(node
            .style_tag_uuids
            .iter()
            .any(|uuid| uuid == "1477f18d-9b3e-4e5c-8047-dc60ba606ddb"));
    }

    #[test]
    fn compile_ir_svg_path_binding_overrides_only_with_a_real_path() {
        // A bound `SvgPath` resolving to a real asset PATH overrides the authored
        // svgPath (the DRAK master-mode weapon icon, driven to `guns.svg`). But the
        // MFD footer's pixel nav arrows bind `SvgPath` to a chrome FillStyle TAG
        // reference (`Tag.<uuid>`) that the icon-preset/asset resolution turns into
        // the real arrow SVG — a raw tag must NOT win over the authored/preset path
        // (else the footer nav arrows vanish; the whole-image guard misses them as
        // they are only a few px).
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.SvgPathBindingTagVsPath",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "icon_tag_binding",
                        "isActive": true,
                        "svgPath": "UI/Textures/Vector/General/CommonIcons/authored_arrow.svg",
                        "svgFill": {"svgPath": "", "renderShape": true, "enableColorOverlay": true, "color": null},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 30.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "icon_path_binding",
                        "isActive": true,
                        "svgPath": "UI/Textures/Vector/General/CommonIcons/authored_placeholder.svg",
                        "svgFill": {"svgPath": "", "renderShape": true, "enableColorOverlay": true, "color": null},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 30.0}
                        }
                    }
                ],
                "operations": [
                    {
                        "_Type_": "BuildingBlocks_BindingsStringField",
                        "widget": "_PointsTo_:ptr:1",
                        "field": "SvgPath",
                        "input": "_PointsTo_:ptr:2"
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_BindingsStringComponentParameter",
                        "name": "tag value",
                        "parameter": "ParamInput0",
                        "defaultValue": "Tag.5616aeff-a713-4530-8004-e7de5d655155"
                    },
                    {
                        "_Type_": "BuildingBlocks_BindingsStringField",
                        "widget": "_PointsTo_:ptr:3",
                        "field": "SvgPath",
                        "input": "_PointsTo_:ptr:4"
                    },
                    {
                        "_Pointer_": "ptr:4",
                        "_Type_": "BuildingBlocks_BindingsStringComponentParameter",
                        "name": "path value",
                        "parameter": "ParamInput1",
                        "defaultValue": "UI/Textures/Vector/General/CommonIcons/guns.svg"
                    }
                ]
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-svg-path-binding-tag-vs-path",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let tag_node = ir.nodes.iter().find(|n| n.name == "icon_tag_binding").expect("tag node");
        assert_eq!(
            tag_node.asset_ref.as_deref(),
            Some("UI/Textures/Vector/General/CommonIcons/authored_arrow.svg"),
            "a tag-reference SvgPath binding must NOT override the authored/preset asset"
        );
        let path_node = ir.nodes.iter().find(|n| n.name == "icon_path_binding").expect("path node");
        assert_eq!(
            path_node.asset_ref.as_deref(),
            Some("UI/Textures/Vector/General/CommonIcons/guns.svg"),
            "a real-path SvgPath binding overrides the authored placeholder"
        );
    }

    #[test]
    fn compile_ir_includes_ancestor_style_tags_on_child_nodes() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.AncestorStyleTags",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetCanvas",
                        "name": "parent",
                        "styleTags": [{"_RecordId_": "bbbbbbbb-cccc-dddd-eeee-ffffffffffff"}],
                        "isActive": true
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "child",
                        "parent": "_PointsTo_:ptr:1",
                        "styleTags": [{"_RecordId_": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"}],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-ancestor-style-tags",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir
            .nodes
            .iter()
            .find(|node| node.name == "child")
            .expect("child node missing");
        assert!(node
            .style_tag_uuids
            .iter()
            .any(|uuid| uuid == "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"));
        assert!(node
            .style_tag_uuids
            .iter()
            .any(|uuid| uuid == "bbbbbbbb-cccc-dddd-eeee-ffffffffffff"));
    }


}

#[cfg(test)]
mod pagein_alpha_tests {
    #![allow(unused_imports, dead_code)]

    use super::*;

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }

    #[test]
    fn compile_ir_settles_pagein_start_root_alpha() {
        // A scene-ROOT node authored alpha=0 + isActive + a page-in `animation`
        // block is the engine's screen page-in container (e.g. m_eng_mfdcontent's
        // `base_Root`). Its settled (post-page-in) alpha is 1.0; otherwise
        // alpha inheritance cascades the 0.0 start value to every descendant and
        // the whole frame renders blank. A different alpha=0 root WITHOUT a
        // page-in animation must stay hidden (no blanket reveal).
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TestPageInRoot",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "base_Root",
                        "isActive": true,
                        "alpha": 0.0,
                        "inheritsAlpha": true,
                        "animation": {"animationTimeline": null, "duration": 1.0, "additive": true},
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        },
                        "children": ["ptr:2"]
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "content",
                        "isActive": true,
                        "parent": "_PointsTo_:ptr:1",
                        "alpha": 1.0,
                        "inheritsAlpha": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 8.0}
                        },
                        "svgFill": {"svgPath": "", "renderShape": true, "enableColorOverlay": true, "strokeExtent": 1.0}
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_DisplayWidget",
                        "name": "static_hidden_root",
                        "isActive": true,
                        "alpha": 0.0,
                        "inheritsAlpha": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 100.0},
                            "height": {"behavior": "Fixed", "value": 100.0}
                        },
                        "children": ["ptr:4"]
                    },
                    {
                        "_Pointer_": "ptr:4",
                        "_Type_": "BuildingBlocks_WidgetSeparator",
                        "name": "hidden_content",
                        "isActive": true,
                        "parent": "_PointsTo_:ptr:3",
                        "alpha": 1.0,
                        "inheritsAlpha": true,
                        "sizing": {
                            "width": {"behavior": "Fixed", "value": 80.0},
                            "height": {"behavior": "Fixed", "value": 8.0}
                        },
                        "svgFill": {"svgPath": "", "renderShape": true, "enableColorOverlay": true, "strokeExtent": 1.0}
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-pagein-root",
            Some("BuildingBlocks_Canvas.TestPageInRoot"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let content = ir.nodes.iter().find(|node| node.name == "content").expect("content node");
        assert!(
            (content.alpha - 1.0).abs() < 0.001,
            "page-in root must settle to 1.0 so descendants are visible; got {}",
            content.alpha
        );
        let hidden = ir.nodes.iter().find(|node| node.name == "hidden_content").expect("hidden node");
        assert!(
            hidden.alpha <= 0.001,
            "an alpha=0 root WITHOUT a page-in animation must NOT be revealed; got {}",
            hidden.alpha
        );
    }
}

#[cfg(test)]
mod tests_f {
    #![allow(unused_imports, dead_code)]

    use super::*;

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }

    #[test]
    fn compile_ir_resolves_primary_state_tag_from_integer_switch_binding() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.PrimaryStateTagSwitch",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "styleTags": [{"_RecordId_": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"}],
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": [
                    {
                        "_Type_": "BuildingBlocks_BindingsStringField",
                        "widget": "_PointsTo_:ptr:1",
                        "field": "PrimaryStateTag",
                        "input": "_PointsTo_:ptr:2"
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_BindingsTagFromIntegerSwitch",
                        "values": [
                            {
                                "first": 1,
                                "second": {
                                    "_RecordName_": "Tag.1477f18d-9b3e-4e5c-8047-dc60ba606ddb",
                                    "_RecordId_": "1477f18d-9b3e-4e5c-8047-dc60ba606ddb"
                                }
                            }
                        ],
                        "defaultValue": null,
                        "input": "_PointsTo_:ptr:3"
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                        "name": "primary state index",
                        "parameter": "ParamInput0",
                        "defaultValue": 1
                    }
                ]
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-primary-state-switch",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        assert!(node
            .style_tag_uuids
            .iter()
            .any(|uuid| uuid == "1477f18d-9b3e-4e5c-8047-dc60ba606ddb"));
    }

    /// A `ComponentLabelCaptionPair` honours its authored component fields:
    /// `labelProperties.show=false` hides the primary label, the caption
    /// applies `captionProperties.caseModifier`, and the pair's `alignment`
    /// field ("Center" — the enum the widget-standard's RootCenter entry
    /// selects on) centres the caption text. The Carrack lift-call console's
    /// `Label_ThisFloor` authors exactly this shape for its floor heading.
    #[test]
    fn compile_ir_label_caption_pair_show_case_and_alignment() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.PairShowCaseAlignment",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_ComponentLabelCaptionPair",
                        "name": "FloorHeading",
                        "isActive": true,
                        "alignment": "Center",
                        "labelProperties": {
                            "show": false,
                            "label": "@test_label_key",
                            "style": "Heading6",
                            "caseModifier": "Upper"
                        },
                        "captionProperties": {
                            "show": true,
                            "caption": "@LOC_PLACEHOLDER",
                            "style": "Heading3",
                            "caseModifier": "Upper"
                        },
                        "size": {
                            "width": {"behavior": "Auto", "value": 64.0},
                            "height": {"behavior": "Auto", "value": 64.0}
                        }
                    }
                ],
                "operations": [
                    {
                        "_Type_": "BuildingBlocks_BindingsStringField",
                        "widget": "_PointsTo_:ptr:1",
                        "field": "ParamInput1",
                        "input": "_PointsTo_:ptr:2"
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_BindingsStringComponentParameter",
                        "name": "floor value",
                        "parameter": "ParamInput1",
                        "defaultValue": "Test Floor"
                    }
                ]
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("test_label_key", "Test Label".to_string());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-pair-show-case-alignment",
            Some("BuildingBlocks_Canvas.PairShowCaseAlignment"),
            (100, 100),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|n| n.name == "FloorHeading").expect("pair node");
        assert_eq!(node.text_payload, None, "show=false hides the primary label");
        assert_eq!(
            node.secondary_text_payload,
            Some(UiIrTextPayload::Resolved { text: "TEST FLOOR".to_string() }),
            "caption applies its authored caseModifier"
        );
        let style = node.secondary_text_style.as_ref().expect("secondary style");
        assert_eq!(style.alignment, "Center", "pair alignment field centres the caption");
    }

    /// A caption authored `show=false` renders no secondary text even when its
    /// bound value resolves.
    #[test]
    fn compile_ir_label_caption_pair_hidden_caption_stays_empty() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.PairHiddenCaption",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_ComponentLabelCaptionPair",
                        "name": "HiddenCaption",
                        "isActive": true,
                        "labelProperties": {
                            "show": true,
                            "label": "@test_label_key",
                            "style": "Heading6",
                            "caseModifier": "None"
                        },
                        "captionProperties": {
                            "show": false,
                            "caption": "@LOC_PLACEHOLDER",
                            "style": "Heading3",
                            "caseModifier": "None"
                        },
                        "size": {
                            "width": {"behavior": "Auto", "value": 64.0},
                            "height": {"behavior": "Auto", "value": 64.0}
                        }
                    }
                ],
                "operations": [
                    {
                        "_Type_": "BuildingBlocks_BindingsStringField",
                        "widget": "_PointsTo_:ptr:1",
                        "field": "ParamInput1",
                        "input": "_PointsTo_:ptr:2"
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_BindingsStringComponentParameter",
                        "name": "hidden value",
                        "parameter": "ParamInput1",
                        "defaultValue": "Hidden Value"
                    }
                ]
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("test_label_key", "Test Label".to_string());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-pair-hidden-caption",
            Some("BuildingBlocks_Canvas.PairHiddenCaption"),
            (100, 100),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = ir.nodes.iter().find(|n| n.name == "HiddenCaption").expect("pair node");
        assert_eq!(node.secondary_text_payload, None, "show=false hides the caption");
        assert!(
            matches!(&node.text_payload, Some(UiIrTextPayload::Resolved { text }) if text == "Test Label"),
            "the shown label still renders: {:?}",
            node.text_payload
        );
    }

    #[test]
    fn compile_ir_suppresses_placeholder_only_label_caption_pairs() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.PlaceholderLabelCaption",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_ComponentLabelCaptionPair",
                        "name": "OperatorName",
                        "isActive": true,
                        "labelProperties": {
                            "label": "@med_Header_OperatorName",
                            "style": "Heading3",
                            "caseModifier": "Upper",
                            "anchorToParentX": 0.5,
                            "anchorToParentY": 0.5
                        },
                        "captionProperties": {
                            "caption": "@LOC_PLACEHOLDER",
                            "style": "Heading6",
                            "caseModifier": "None"
                        },
                        "size": {
                            "width": {"behavior": "Auto", "value": 64.0},
                            "height": {"behavior": "Auto", "value": 64.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("med_header_operatorname", "OPERATOR NAME".to_string());
        defaults.insert_localization("loc_placeholder", String::new());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-placeholder",
            None,
            (100, 100),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        assert_eq!(ir.nodes.len(), 1);
        let node = &ir.nodes[0];
        assert!(!node.is_active, "placeholder-only label-caption pair should compile inactive");
        assert!(matches!(node.text_payload, Some(UiIrTextPayload::Resolved { .. })));
        assert!(node.secondary_text_payload.is_none());
    }

    #[test]
    fn compile_ir_secondary_param_input_loc_empty_uses_unified_empty_state() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.LabelCaptionLocEmpty",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_ComponentLabelCaptionPair",
                        "name": "OperatorName",
                        "isActive": true,
                        "labelProperties": {
                            "label": "OPERATOR",
                            "style": "Heading3",
                            "caseModifier": "Upper"
                        },
                        "captionProperties": {
                            "caption": "@LOC_EMPTY",
                            "style": "Heading6",
                            "caseModifier": "None"
                        },
                        "size": {
                            "width": {"behavior": "Auto", "value": 64.0},
                            "height": {"behavior": "Auto", "value": 64.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("loc_empty", String::new());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-secondary-loc-empty",
            None,
            (100, 100),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        assert!(node.secondary_text_payload.is_none());
        assert!(ir
            .warnings
            .iter()
            .all(|warning| !warning.contains("unresolved text key")));
    }

    #[test]
    fn compile_ir_resolves_localization_combine_binding_chain_for_text_field() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.LocalizationCombineBinding",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "text_TierValue",
                        "labelProperties": {
                            "style": "Heading3",
                            "caseModifier": "Upper"
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 12.0}
                        }
                    }
                ],
                "operations": [
                    {
                        "_Type_": "BuildingBlocks_BindingsLocalizedField",
                        "widget": "_PointsTo_:ptr:1",
                        "field": "ParamInput0",
                        "input": "_PointsTo_:ptr:2"
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BindingsOperations_LocalizationCombine",
                        "value": "@Med_T_Tier",
                        "inputL": null,
                        "inputR": "_PointsTo_:ptr:3"
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_BindingsIntegerComponentParameter",
                        "name": "tier",
                        "parameter": "ParamInput0",
                        "defaultValue": 3
                    }
                ]
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("med_t_tier", "Tier %d".to_string());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-localization-combine",
            None,
            (100, 100),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        assert_eq!(
            node.text_payload,
            Some(UiIrTextPayload::Resolved {
                text: "TIER 3".to_string(),
            })
        );
        assert!(ir
            .warnings
            .iter()
            .all(|warning| !warning.contains("unresolved text key")));
    }

    #[test]
    fn compile_ir_preserves_text_fidelity_signals_for_case_font_size_and_colour() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.TextFidelitySignals",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "labelProperties": {
                            "label": "@Info_Kiosks_LogoScreen_001",
                            "caseModifier": "Upper"
                        },
                        "FontStyleRecord": "record://fonts/title",
                        "FontSize": 14.0,
                        "FillColor": {"r": 0.25, "g": 0.5, "b": 0.75, "a": 1.0},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 12.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("info_kiosks_logoscreen_001", "Touch to start".to_string());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-text-fidelity",
            Some("BuildingBlocks_Canvas.TextFidelitySignals"),
            (100, 100),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        assert_eq!(
            node.text_payload,
            Some(UiIrTextPayload::Resolved {
                text: "TOUCH TO START".to_string(),
            })
        );
        let text_style = node.text_style.as_ref().expect("text style");
        assert_eq!(text_style.font_record.as_deref(), Some("record://fonts/title"));
        // caseModifier Upper no longer applies an all-caps size reduction; the authored
        // 14.0 is used verbatim.
        assert!(matches!(text_style.font_size, UiIrValue::Fixed { value } if (value - 14.0).abs() < 0.01));
        assert_eq!(text_style.colour, Some([0.25, 0.5, 0.75, 1.0]));
    }

    #[test]
    fn compile_ir_prefers_raw_font_size_over_stale_parsed_text_size() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.RawFontSize",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let node = scene.nodes.get_mut(&1).expect("node 1");
        node.raw["FontSize"] = serde_json::Value::from(42.0);

        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-raw-font-size",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 42.0 });
    }

    #[test]
    fn compile_ir_widget_text_preserves_loc_string_case_colour_and_auto_font_size() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.WidgetTextStatus",
            "_RecordValue_": {
                "size": {"x": 200, "y": 200},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetText",
                        "name": "status",
                        "locString": "@mykey",
                        "color": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Base",
                            "alpha": 1.0
                        },
                        "fontStyle": "file://./../../../../../../../libs/foundry/records/ui/buildingblocks/fontstyles/audimatmono-regular.json",
                        "autoFontSize": true,
                        "fontSize": 16.0,
                        "labelProperties": {
                            "caseModifier": "Upper"
                        },
                        "size": {
                            "width": {"behavior": "Fixed", "value": 120.0},
                            "height": {"behavior": "Fixed", "value": 80.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("mykey", "Closed".to_string());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-widget-text-status",
            None,
            (200, 200),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        assert_eq!(
            node.text_payload,
            Some(UiIrTextPayload::Resolved {
                text: "CLOSED".to_string(),
            })
        );
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(
            style.font_record.as_deref(),
            Some("file://./../../../../../../../libs/foundry/records/ui/buildingblocks/fontstyles/audimatmono-regular.json")
        );
        assert_eq!(style.resolved_font_record, None);
        assert_eq!(style.colour_token.as_deref(), Some("Base"));
        // Data-backed model: autoFontSize growth is render-side, so the IR carries the
        // flag and an authored seed size (not a heuristic value).
        assert!(node.auto_font_size, "autoFontSize node must carry the grow flag");
        assert!(matches!(style.font_size, UiIrValue::Fixed { .. }));
    }
}

#[cfg(test)]
mod tests_g {
    #![allow(unused_imports, dead_code)]

    use super::*;

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }

    fn font_record_with_percent(font: &str, image_size_percent: f32) -> serde_json::Value {
        serde_json::json!({
            "_RecordValue_": {
                "font": font,
                "imageSizePercent": image_size_percent
            }
        })
    }

    #[test]
    fn image_size_percent_compensation_applies_for_med_heavy() {
        let record = font_record_with_percent("$Med-Heavy", 0.75);
        let adjusted = adjust_ui_ir_font_value_for_font_record_image_percent(
            UiIrValue::Fixed { value: 64.0 },
            Some(&record),
        );
        assert!(matches!(adjusted, UiIrValue::Fixed { value } if (value - 85.333336).abs() < 0.001));
    }

    #[test]
    fn image_size_percent_compensation_applies_for_any_font_with_percent() {
        // Data-backed model: the imageSizePercent boost is per-font (it is how the
        // engine renders plain text larger than its nominal), not gated to one font.
        // Any font carrying `imageSizePercent` is compensated: 64 / 0.75 = 85.33.
        let record = font_record_with_percent("$Text1Thin", 0.75);
        let adjusted = adjust_ui_ir_font_value_for_font_record_image_percent(
            UiIrValue::Fixed { value: 64.0 },
            Some(&record),
        );
        assert!(matches!(adjusted, UiIrValue::Fixed { value } if (value - 85.333336).abs() < 0.001));
    }


    #[test]
    fn compile_ir_widget_text_auto_font_size_defers_growth_to_renderer() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.WidgetTextLargeSlot",
            "_RecordValue_": {
                "size": {"x": 1920, "y": 1080},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetText",
                        "name": "status",
                        "locString": "@mykey",
                        "color": {
                            "_Type_": "BuildingBlocks_ColorStyle",
                            "color": "Base",
                            "alpha": 1.0
                        },
                        "autoFontSize": true,
                        "fontSize": 16.0,
                        "textAlignment": "Center",
                        "verticalAlignment": "Center",
                        "wordWrap": false,
                        "caseModifier": "Upper",
                        "sizing": {
                            "_Type_": "BuildingBlocks_Size",
                            "width": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "behavior": "Percent", "value": 0.6},
                            "height": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "behavior": "PercentOfX", "value": 0.3},
                            "depth": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "behavior": "Fixed", "value": 0.0},
                            "minWidth": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "behavior": "Fixed", "value": 0.0},
                            "minHeight": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "behavior": "Fixed", "value": 0.0},
                            "maxWidth": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "behavior": "Fixed", "value": 0.0},
                            "maxHeight": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "behavior": "Fixed", "value": 0.0},
                            "enableMinWidth": false,
                            "enableMinHeight": false,
                            "enableMaxWidth": false,
                            "enableMaxHeight": false
                        },
                        "anchor": {"x": 0.5, "y": 0.6, "z": 0.0},
                        "pivot": {"x": 0.5, "y": 0.5, "z": 0.0}
                    }
                ],
                "operations": []
            }
        });

        let mut defaults = defaults();
        defaults.insert_localization("mykey", "Closed".to_string());

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-widget-text-large-slot",
            None,
            (1920, 1080),
            &defaults,
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        // Data-backed model: the IR no longer applies a heuristic growth/cap to
        // autoFontSize text — it carries the `auto_font_size` flag and the authored
        // seed size (16, no font record here to boost), and the renderer fits the
        // glyphs to the rect with real metrics. Runaway growth is bounded there by the
        // rect, so the IR value stays the authored size.
        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert!(node.auto_font_size, "autoFontSize node must carry the grow flag");
        assert!(
            matches!(style.font_size, UiIrValue::Fixed { value } if (15.9..=16.1).contains(&value)),
            "autoFontSize IR keeps the authored seed size, growth is render-side: {:?}",
            style.font_size
        );
    }

    #[test]
    fn compile_ir_prefers_raw_font_record_over_stale_parsed_text_font_record() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.RawFontRecord",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "fontRecord": "file://./old-font.json",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let mut scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let node = scene.nodes.get_mut(&1).expect("node 1");
        node.raw["FontStyleRecord"] =
            serde_json::Value::from("file://./styled-font.json");

        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-raw-font-record",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(
            style.font_record.as_deref(),
            Some("file://./styled-font.json")
        );
    }

    #[test]
    fn compile_ir_uses_scene_style_font_size_when_node_font_size_is_missing() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.StyleFontSizeFallback",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "reference",
                        "text": "REF",
                        "FontSize": 40.0,
                        "labelProperties": {"style": "Heading1"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "target",
                        "text": "TARGET",
                        "labelProperties": {"style": "Heading1"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-style-font-size-fallback",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let target = ir
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .expect("target node");
        let style = target.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 40.0 });
    }

    #[test]
    fn compile_ir_reads_font_size_from_modifiers_projection() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.ModifierFontSize",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "modifiers": {"FontSize": 50.0},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-modifier-font-size",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 50.0 });
    }

    #[test]
    fn compile_ir_inherits_label_style_from_ancestor_for_font_size_fallback() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.AncestorStyleFallback",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "reference",
                        "text": "REF",
                        "FontSize": 40.0,
                        "labelProperties": {"style": "Heading1"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_ComponentDisplayWidget",
                        "name": "parent",
                        "labelProperties": {"style": "Heading1"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:3",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "target",
                        "text": "TARGET",
                        "parent": "_PointsTo_:ptr:2",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 40.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-ancestor-style-fallback",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        let target = ir
            .nodes
            .iter()
            .find(|node| node.name == "target")
            .expect("target node");
        let style = target.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 40.0 });
    }

    #[test]
    fn compile_ir_unsized_textfield_without_brand_table_falls_through_to_parsed_size() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.FontSizeException",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "labelProperties": {"style": "Heading3"},
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-font-size-exception",
            None,
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        // Data-backed model: with no brand-style table available (no fetcher) for this
        // Heading3 textfield, there is no hard-coded font-size exception any more — it
        // falls through to the size parsed from the widget itself (12.0). In a real
        // render the fetcher supplies the Heading3 brand FontSize via the STYLE branch.
        let node = &ir.nodes[0];
        let style = node.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size, UiIrValue::Fixed { value: 12.0 });
    }

    #[test]
    fn compile_ir_without_selected_swf_source_uses_bb_renderer_hint() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.CustomShapeNoSwf",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "shape",
                        "shapeType": "line",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-custom-shape-no-swf",
            Some("BuildingBlocks_Canvas.CustomShapeNoSwf"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        assert_eq!(ir.renderer_hint, UiRendererHint::Bb);
        assert!(ir
            .warnings
            .iter()
            .any(|warning| warning.contains("no SWF source was resolved")));
    }

    #[test]
    fn compile_ir_with_selected_swf_source_preserves_hybrid_renderer_hint() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.CustomShapeWithSwf",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "READY",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    },
                    {
                        "_Pointer_": "ptr:2",
                        "_Type_": "BuildingBlocks_WidgetCustomShape",
                        "name": "shape",
                        "shapeType": "line",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 20.0},
                            "height": {"behavior": "Fixed", "value": 20.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-custom-shape-with-swf",
            Some("BuildingBlocks_Canvas.CustomShapeWithSwf"),
            (100, 100),
            &defaults(),
            None,
            Some("Data\\UI\\ShipInterface\\assets\\SWF\\test.swf".to_string()),
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        assert_eq!(ir.renderer_hint, UiRendererHint::Hybrid);
    }


}

#[cfg(test)]
mod tests_h {
    #![allow(unused_imports, dead_code)]

    use super::*;

    struct TestCanvasFetcher {
        by_guid: std::collections::HashMap<String, serde_json::Value>,
        by_path: std::collections::HashMap<String, serde_json::Value>,
    }

    impl CanvasFetcher for TestCanvasFetcher {
        fn fetch_canvas_json(&self, guid: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .get(guid)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing guid: {guid}")))
        }

        fn fetch_canvas_by_name(
            &self,
            record_name: &str,
        ) -> Result<serde_json::Value, crate::UiError> {
            self.by_guid
                .values()
                .chain(self.by_path.values())
                .find(|value| {
                    value
                        .get("_RecordName_")
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == record_name)
                })
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing name: {record_name}")))
        }

        fn fetch_canvas_by_path(&self, path: &str) -> Result<serde_json::Value, crate::UiError> {
            self.by_path
                .get(path)
                .cloned()
                .ok_or_else(|| crate::UiError::RenderError(format!("missing path: {path}")))
        }
    }

    fn defaults() -> crate::defaults::DefaultValueRegistry {
        crate::defaults::DefaultValueRegistry::with_well_known_path_defaults()
    }


    #[test]
    fn compile_ir_treats_loc_empty_as_intentionally_empty_not_unresolved() {
        let canvas = serde_json::json!({
            "_RecordName_": "BuildingBlocks_Canvas.IntentionallyEmptyText",
            "_RecordValue_": {
                "size": {"x": 100, "y": 100},
                "scene": [
                    {
                        "_Pointer_": "ptr:1",
                        "_Type_": "BuildingBlocks_WidgetTextField",
                        "name": "label",
                        "text": "@LOC_EMPTY",
                        "size": {
                            "width": {"behavior": "Fixed", "value": 30.0},
                            "height": {"behavior": "Fixed", "value": 10.0}
                        }
                    }
                ],
                "operations": []
            }
        });

        let scene = crate::bb_scene::parse_bb_canvas(&canvas).expect("scene parse");
        let ir = compile_ui_ir_from_scene(
            &scene,
            None,
            "guid-intentionally-empty",
            Some("BuildingBlocks_Canvas.IntentionallyEmptyText"),
            (100, 100),
            &defaults(),
            None,
            None,
            &[],
            Vec::new(),
            Vec::new(),
            100,
        );

        assert_eq!(
            ir.nodes[0].text_payload,
            Some(UiIrTextPayload::IntentionallyEmpty {
                key: Some("@LOC_EMPTY".to_string()),
            })
        );
        assert!(ir
            .warnings
            .iter()
            .all(|warning| !warning.contains("unresolved text key")));
        assert_eq!(ir.confidence, 100);
    }

    #[test]
    fn ui_ir_source_does_not_reintroduce_forbidden_hardcoded_markers() {
        // Scan the REAL engine sources. (The pre-F1 version read `engine.inc`,
        // which only held `include!` directives — the guard was silently
        // vacuous; review F1 made it scan the module files themselves.)
        let sources = [
            include_str!("engine_01.rs"),
            include_str!("engine_02.rs"),
            include_str!("engine_03.rs"),
            include_str!("engine_04.rs"),
        ];
        let forbidden = [
            ["nominal_font_size_", "from_label_style"].concat(),
            ["BG", "Dots"].concat(),
            ["MainMenu", "Canvas"].concat(),
            ["base_", "animatedelements"].concat(),
            ["apply_medical", "attract_banner_layout"].concat(),
        ];

        for marker in forbidden {
            assert!(
                !sources.iter().any(|source| source.contains(marker.as_str())),
                "ui_ir hardcoding marker reintroduced: {marker}"
            );
        }
    }
}
