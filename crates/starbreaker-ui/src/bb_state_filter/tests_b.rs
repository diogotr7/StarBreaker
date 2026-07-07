    use super::*;
    use serde_json::json;

    fn boolean_field_op(widget_ptr: u32, input_ptr: u32) -> serde_json::Value {
        json!({
            "_Type_": "BuildingBlocks_BindingsBooleanField",
            "widget": format!("_PointsTo_:ptr:{widget_ptr}"),
            "field": "Instantiated",
            "input": format!("_PointsTo_:ptr:{input_ptr}")
        })
    }

    fn variable_op(ptr: u32, binding: &str) -> serde_json::Value {
        json!({
            "_Pointer_": format!("ptr:{ptr}"),
            "_Type_": "BuildingBlocks_BindingsBooleanVariable",
            "binding": binding
        })
    }

    fn static_var(name: &str, val: bool) -> serde_json::Value {
        json!({ "name": name, "value": val })
    }

    fn make_record_value(
        static_vars: Vec<serde_json::Value>,
        ops: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        json!({
            "_Type_": "BuildingBlocks_Canvas",
            "staticVariables": static_vars,
            "operations": ops
        })
    }

    // ── test 1 ──────────────────────────────────────────────────────────────

    /// Canvas with no operations produces an empty false set.
    #[test]
    fn direct_variable_scene_order_picks_first_as_cold_default() {
        // ptr:3 = Attract, ptr:4 = MainMenu, ptr:7 = Heal
        // ptr:5 (AttractCanvas) bound to ptr:3
        // ptr:6 (MainMenuCanvas) bound to ptr:4
        // ptr:8 (HealCanvas) bound to ptr:7
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.BaseScreens.Attract"),
                variable_op(4, "state.BaseScreens.MainMenu"),
                variable_op(7, "state.BaseScreens.Heal"),
                boolean_field_op(5, 3),
                boolean_field_op(6, 4),
                boolean_field_op(8, 7),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "AttractCanvas (ptr:5) shown — first direct-variable in group is cold-default");
        assert!(result.contains(&6), "MainMenuCanvas (ptr:6) hidden — not the cold-default");
        assert!(result.contains(&8), "HealCanvas (ptr:8) hidden — not the cold-default");
    }

    /// Bed base-screen canvases should prefer MainMenu as cold-default when no
    /// explicit static override exists.
    #[test]
    fn bed_base_screens_prefers_mainmenu() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "Bed/state.BaseScreens.Attract"),
                variable_op(4, "Bed/state.BaseScreens.MainMenu"),
                variable_op(7, "Bed/state.BaseScreens.Heal"),
                boolean_field_op(5, 3), // AttractCanvas
                boolean_field_op(6, 4), // MainMenuCanvas
                boolean_field_op(8, 7), // HealCanvas
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "AttractCanvas (ptr:5) hidden for Bed default");
        assert!(
            !result.contains(&6),
            "MainMenuCanvas (ptr:6) shown as Bed cold-default"
        );
        assert!(result.contains(&8), "HealCanvas (ptr:8) hidden");
    }

    /// Direct-variable rule requires a group of ≥2 same-prefix variables.
    /// A single-member group (just `state.X` with no siblings) must NOT be
    /// elected — it's likely a single hide/show flag, not a state-machine.
    #[test]
    fn direct_variable_singleton_group_not_promoted() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.LonelyFlag"),
                boolean_field_op(5, 3),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&5),
            "Singleton-group canvas (ptr:5) must remain filtered — no other group members to make it a state-machine"
        );
    }


    /// If an `Invert(Or(...))` gate names directly-gated sibling canvases,
    /// the first Or operand remains the idle overlay and the framing canvas
    /// follows its evaluated `Instantiated` value (hidden when false).
    #[test]
    fn idle_gate_or_filters_framing_canvas_when_false() {
        // Mirrors the wall medbay shape:
        // ptr:3 = Attract, ptr:4 = LogIn, ptr:19 = Or(3, 4), ptr:6 = NOT(19)
        // ptr:5 (Header) bound to ptr:6 → hidden when Attract is cold-default
        // ptr:11 (LogInCanvas) bound to ptr:4 → hidden
        // ptr:8 (AttractCanvas) bound to ptr:3 → shown as first Or operand
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Attract"),
                variable_op(4, "state.LogIn"),
                json!({
                    "_Pointer_": "ptr:19",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:3", "_PointsTo_:ptr:4"]
                }),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:19"
                }),
                boolean_field_op(5, 6),
                // Direct-field order in the real wall canvas is LogIn, then Attract.
                boolean_field_op(11, 4),
                boolean_field_op(8, 3),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Header (ptr:5) hidden when Invert(Or(...)) evaluates false");
        assert!(result.contains(&11), "LogInCanvas (ptr:11) hidden (LogIn=false)");
        assert!(!result.contains(&8), "AttractCanvas (ptr:8) shown (first Or operand)");
    }

    /// `Invert(Or(...))` still falls back to the first Or operand when the Or
    /// operands do not have directly-gated sibling canvases.
    #[test]
    fn idle_gate_or_without_direct_siblings_uses_first_operand() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Attract"),
                variable_op(4, "state.LogIn"),
                json!({
                    "_Pointer_": "ptr:19",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateOr",
                    "inputs": ["_PointsTo_:ptr:3", "_PointsTo_:ptr:4"]
                }),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:19"
                }),
                boolean_field_op(5, 6),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "Header (ptr:5) hidden under first-operand idle default");
    }

    /// Explicit static-true override of any group member SUPPRESSES the
    /// idle-default rule: the idle-gate variable stays false and the
    /// framing widget is shown.
    #[test]
    fn explicit_group_override_suppresses_idle_default() {
        // ptr:3 = Attract, ptr:7 = Admin, ptr:6 = NOT(3)
        // staticVariables[]: state.Admin=true (explicit override → suppresses
        // Attract idle-default)
        let rv = make_record_value(
            vec![static_var("state.Admin", true)],
            vec![
                variable_op(3, "state.Attract"),
                variable_op(7, "state.Admin"),
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:3"
                }),
                boolean_field_op(5, 6),
                boolean_field_op(8, 7),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "Header (ptr:5) shown — Admin override suppresses Attract idle-default");
        assert!(!result.contains(&8), "AdminCanvas (ptr:8) shown via explicit static-true");
    }

    /// Old test 6 kept as a separate scenario: when the inverted variable is
    /// NOT a member of an idle-gate group (no shared dotted prefix at all),
    /// idle-default does not kick in and `NOT(false) → true`.
    #[test]
    fn ungrouped_invert_does_not_trigger_idle_default() {
        // Variable binding is a single segment with no dotted prefix.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "Attract"), // no `.` → no group
                json!({
                    "_Pointer_": "ptr:6",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:3"
                }),
                boolean_field_op(5, 6),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(!result.contains(&5), "ungrouped variable: NOT(false)=true, Header shown");
    }

    #[test]
    fn is_active_false_widget_is_filtered() {
        // `IsActive` is treated as a visibility field just like `Instantiated`.
        // Variable `state.Foo` has no static default → false → widget ptr:5 hidden.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.Foo"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:5",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&5), "IsActive=false widget (ptr:5) must be filtered");
    }

    #[test]
    fn visible_false_widget_is_filtered() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.ActorIsInBed"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:7",
                    "field": "Visible",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(result.contains(&7), "Visible=false widget (ptr:7) must be filtered");
    }

    #[test]
    fn unknown_field_is_not_filtered() {
        // Bindings to non-visibility fields (e.g. `Text`, `Color`) must not
        // cause widget filtering.
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(3, "state.SomeFlag"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:9",
                    "field": "Text",
                    "input": "_PointsTo_:ptr:3"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&9),
            "Bindings to non-visibility fields must not filter the widget"
        );
    }

    #[test]
    fn boolean_component_parameter_default_false_hides_is_active_without_override() {
        let rv = make_record_value(
            vec![],
            vec![serde_json::json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:5",
                "field": "IsActive",
                "input": {
                    "_Pointer_": "ptr:9",
                    "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                    "parameter": "ParamInput0",
                    "defaultValue": false
                }
            })],
        );

        let no_override = instantiated_false_widgets_with_param_inputs(&rv, &[]);
        assert!(
            no_override.contains(&5),
            "without paramInput override, explicit defaultValue=false should hide ptr:5"
        );

        let with_false_override = instantiated_false_widgets_with_param_inputs(
            &rv,
            &[serde_json::json!({
                "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
                "parameter": "ParamInput0",
                "value": false
            })],
        );
        assert!(
            with_false_override.contains(&5),
            "explicit false paramInput override should hide ptr:5"
        );

        let with_override = instantiated_false_widgets_with_param_inputs(
            &rv,
            &[serde_json::json!({
                "_Type_": "BuildingBlocks_ComponentParameterInputBoolean",
                "parameter": "ParamInput0",
                "value": true
            })],
        );
        assert!(
            !with_override.contains(&5),
            "paramInput override true should show ptr:5"
        );
    }

    #[test]
    fn boolean_component_parameter_missing_override_without_default_stays_visible() {
        let rv = make_record_value(
            vec![],
            vec![serde_json::json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:5",
                "field": "IsActive",
                "input": {
                    "_Pointer_": "ptr:9",
                    "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                    "parameter": "ParamInput0"
                }
            })],
        );

        let no_override = instantiated_false_widgets_with_param_inputs(&rv, &[]);
        assert!(
            !no_override.contains(&5),
            "without paramInput override and no defaultValue, IsActive should remain visible"
        );
    }

    #[test]
    fn non_state_variables_without_static_values_do_not_hide_widgets() {
        let rv = make_record_value(
            vec![],
            vec![
                json!({
                    "_Pointer_": "ptr:12",
                    "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                    "binding": "CloneLocationInfo/UserOwnsLocation"
                }),
                json!({
                    "_Pointer_": "ptr:13",
                    "_Type_": "BuildingBlocks_BindingsBooleanVariable",
                    "binding": "Bed/MedBed/MedBedStatus/CanRespawnHere"
                }),
                json!({
                    "_Pointer_": "ptr:8",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateAnd",
                    "inputs": ["_PointsTo_:ptr:12", "_PointsTo_:ptr:13"]
                }),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:7",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:8"
                }),
            ],
        );
        let result = instantiated_false_widgets(&rv);
        assert!(
            !result.contains(&7),
            "non-state sensor variables without static defaults should not hide ptr:7"
        );
    }

    /// A boolean component parameter NAMED after an authored static variable
    /// takes the variable's authored static value over its editor
    /// `defaultValue` — the power screen's notification overlays
    /// (`engineeringoverride`/`presetnotification`, editor default `true`)
    /// are authored `staticVariables = false` for the at-rest state.
    #[test]
    fn boolean_component_parameter_takes_static_variable_named_default() {
        let rv = make_record_value(
            vec![json!({
                "_Type_": "BuildingBlocks_StaticVariableBoolean",
                "name": "engineeringoverride",
                "value": false
            })],
            vec![json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:5",
                "field": "IsActive",
                "input": {
                    "_Pointer_": "ptr:9",
                    "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                    "name": "engineeringoverride",
                    "parameter": "ParamInput3",
                    "defaultValue": true
                }
            })],
        );

        let false_set = instantiated_false_widgets_with_param_inputs(&rv, &[]);
        assert!(
            false_set.contains(&5),
            "static variable false must gate the overlay despite editor defaultValue true"
        );
    }

    /// The static-variable name match is case-insensitive (gen params are
    /// lower-cased `presetnotification` while the master variable is
    /// `PresetNotification`).
    #[test]
    fn boolean_component_parameter_static_variable_match_is_case_insensitive() {
        let rv = make_record_value(
            vec![json!({
                "_Type_": "BuildingBlocks_StaticVariableBoolean",
                "name": "PresetNotification",
                "value": false
            })],
            vec![json!({
                "_Type_": "BuildingBlocks_BindingsBooleanField",
                "widget": "_PointsTo_:ptr:6",
                "field": "Instantiated",
                "input": {
                    "_Pointer_": "ptr:9",
                    "_Type_": "BuildingBlocks_BindingsBooleanComponentParameter",
                    "name": "presetnotification",
                    "parameter": "ParamInput2",
                    "defaultValue": true
                }
            })],
        );

        let false_set = instantiated_false_widgets_with_param_inputs(&rv, &[]);
        assert!(false_set.contains(&6), "case-insensitive name match must apply");
    }

    /// Cockpit radar (`MapDisplayMaster`, `binding_kind = radar`) must select
    /// the radar/local RTT mode at static rest. The `/MapNamespace` mode flags,
    /// pinned in the default-value registry, gate the master canvas's per-mode
    /// sub-displays:
    ///   InteriorMapDisplay = And(¬IsRTT, IsInteriorMapActive)   → must hide
    ///   StarMapDisplayRTT  = And(IsRTT,  IsStarMapActive)        → must stay
    /// (`StarMapDisplayRTT` hosts the `PlayerRadarPlane` radar scope.) Without
    /// the registry pins these `/MapNamespace` BooleanVariables are unset
    /// non-state bindings, so the `contains_unset_non_state_variable` override
    /// forces every mode active and the interior-map chrome over-paints the
    /// radar screen. The flag values live in the registry, not in code.
    #[test]
    fn radar_mode_registry_defaults_select_starmap_rtt_over_interior_map() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(25, "/~/MapNamespace~/GeneralMapData/IsRTT"),
                variable_op(26, "/~/MapNamespace~GeneralMapData/IsInteriorMapActive"),
                variable_op(31, "/~/MapNamespace~/GeneralMapData/IsStarMapActive"),
                json!({
                    "_Pointer_": "ptr:27",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:25"
                }),
                // InteriorMapDisplay (ptr:15) = And(¬IsRTT, IsInteriorMapActive)
                json!({
                    "_Pointer_": "ptr:16",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateAnd",
                    "inputs": ["_PointsTo_:ptr:27", "_PointsTo_:ptr:26"]
                }),
                // StarMapDisplayRTT (ptr:33) = And(IsRTT, IsStarMapActive)
                json!({
                    "_Pointer_": "ptr:34",
                    "_Type_": "BuildingBlocks_BindingsBooleanEvaluateAnd",
                    "inputs": ["_PointsTo_:ptr:25", "_PointsTo_:ptr:31"]
                }),
                boolean_field_op(15, 16),
                boolean_field_op(33, 34),
            ],
        );
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let false_set =
            instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
                &rv,
                &[],
                &std::collections::HashMap::new(),
                Some(&defaults),
            );
        assert!(
            false_set.contains(&15),
            "InteriorMapDisplay must deactivate in radar mode (¬IsRTT ∧ IsInteriorMapActive = false)"
        );
        assert!(
            !false_set.contains(&33),
            "StarMapDisplayRTT (radar-plane host) must stay active (IsRTT ∧ IsStarMapActive = true)"
        );
    }

    /// The MFD-radar readout bar (`mapdisplaystarmap_radarreadouts`) shows a
    /// `LockedIcon` whose `IsActive` binds `StarMapData/ShowRadarLocked`. At
    /// static rest the radar is operational (the reference shows the heading /
    /// range readout, no lock), so the registry pins `ShowRadarLocked = false`
    /// and the lock must deactivate. Without the pin it is an unset non-state
    /// binding → the override keeps it active → a spurious padlock over the
    /// readout.
    #[test]
    fn radar_locked_icon_hidden_when_show_radar_locked_pinned_false() {
        let rv = make_record_value(
            vec![],
            vec![
                variable_op(50, "StarMapData/ShowRadarLocked"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:19",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:50"
                }),
            ],
        );
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let false_set =
            instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
                &rv,
                &[],
                &std::collections::HashMap::new(),
                Some(&defaults),
            );
        assert!(
            false_set.contains(&19),
            "LockedIcon must deactivate when ShowRadarLocked is pinned false"
        );
    }

    /// The radar background `image_Background` (DRAK_GroundVehicle_Dashboard bg)
    /// is authored `isActive=false` with `IsActive ← NOT(IsVolumetric)`. At the
    /// flat radar (IsVolumetric pinned false → NOT = genuine Some(true)) it must
    /// be force-activated. A node gated on an UNSET sensor (eval None, override
    /// would make it "true" for the false-set path) must NOT be force-activated.
    #[test]
    fn forced_active_activates_genuine_isactive_true_only() {
        // ptr:5 = a WidgetImage (the radar background), ptr:6 = a WidgetImage gated
        // on an unset sensor, ptr:7 = a DisplayWidget gated genuine-true (the
        // medical-bed class — must NOT activate, scope is WidgetImage-only).
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "scene": [
                {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_WidgetImage", "name": "bg"},
                {"_Pointer_": "ptr:6", "_Type_": "BuildingBlocks_WidgetImage", "name": "other"},
                {"_Pointer_": "ptr:7", "_Type_": "BuildingBlocks_DisplayWidget", "name": "med"},
            ],
            "operations": [
                variable_op(10, "/~/MapNamespace~/GeneralMapData/IsVolumetric"),
                json!({
                    "_Pointer_": "ptr:11",
                    "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                    "input": "_PointsTo_:ptr:10"
                }),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:5",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:11"
                }),
                // gated on an unset sensor var → genuine eval None
                variable_op(20, "SomeUnsetSensor/Live"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:6",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:20"
                }),
                // a DisplayWidget gated genuine-true (NOT IsVolumetric) — excluded
                // by the WidgetImage scope.
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:7",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:11"
                }),
            ],
        });
        let defaults = crate::defaults::DefaultValueRegistry::with_well_known_path_defaults();
        let active = forced_active_widgets_with_defaults(
            &rv,
            &[],
            &std::collections::HashMap::new(),
            Some(&defaults),
        );
        assert!(
            active.contains(&5),
            "WidgetImage background (NOT IsVolumetric, pinned false → Some(true)) must be force-activated"
        );
        assert!(
            !active.contains(&6),
            "unset-sensor IsActive (genuine eval None) must NOT be force-activated"
        );
        assert!(
            !active.contains(&7),
            "a DisplayWidget genuine-true on a non-flight-controller binding (IsVolumetric) must NOT be force-activated"
        );
    }

    #[test]
    fn forced_active_activates_flight_controller_display_widget_not_bed_state() {
        // The LR-indicator CPLD/ESP/LOCK indicators are authored-false
        // `DisplayWidget`s whose `IsActive` binds the ship's FLIGHT-CONTROLLER
        // state (coupled/ESP/…), genuinely lit at the at-rest power-on state — they
        // MUST force-activate. A medical-bed `DisplayWidget` binds `Bed/state.*`
        // (genuine-true at rest too, but the `ui_target_a` gold baseline shows it
        // inactive) and must NOT — the workflow §10 hazard the WidgetImage scope
        // previously guarded by excluding ALL DisplayWidgets.
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "scene": [
                {"_Pointer_": "ptr:5", "_Type_": "BuildingBlocks_DisplayWidget", "name": "esp"},
                {"_Pointer_": "ptr:6", "_Type_": "BuildingBlocks_DisplayWidget", "name": "bed"},
            ],
            "operations": [
                variable_op(10, "flightcontroller.isesp"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:5",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:10"
                }),
                variable_op(20, "Bed/state.BaseScreens.Attract"),
                json!({
                    "_Type_": "BuildingBlocks_BindingsBooleanField",
                    "widget": "_PointsTo_:ptr:6",
                    "field": "IsActive",
                    "input": "_PointsTo_:ptr:20"
                }),
            ],
        });
        let mut inherited = std::collections::HashMap::new();
        inherited.insert("flightcontroller.isesp".to_string(), true);
        inherited.insert("Bed/state.BaseScreens.Attract".to_string(), true);
        let active = forced_active_widgets_with_defaults(&rv, &[], &inherited, None);
        assert!(
            active.contains(&5),
            "a flight-controller-grounded DisplayWidget (LR-indicator ESP) must force-activate"
        );
        assert!(
            !active.contains(&6),
            "a Bed/state-grounded DisplayWidget (medical bed) must NOT force-activate"
        );
    }

    /// Two sibling canvas variants gated on a mutually-exclusive toggle (`X` and
    /// `NOT X`) whose selector is UNSET at static rest must BOTH stay instantiated
    /// — with no value we can't pick a mode, so the static export composites both
    /// authored variants. Motivating case: the cockpit radar's host-planes —
    /// `HostplaneVisuals_Large.Instantiated = StarMapData/CommonData/IsFullScreen`
    /// and `HostplaneVisuals_Small.Instantiated = NOT IsFullScreen`, with
    /// `IsFullScreen` unset (a `/`-path toggle that escapes idle-default grouping).
    /// Without the rule the direct (`X`) side is wrongly deactivated while the
    /// inverted (`NOT X`) side is kept by the unset-override — asymmetric.
    #[test]
    fn unset_mutually_exclusive_instantiation_toggle_keeps_both_variants() {
        // ptr:3 = IsFullScreen; ptr:11 = NOT IsFullScreen.
        // ptr:2 (Large) Instantiated = ptr:3 (direct); ptr:4 (Small) = ptr:11.
        // Both are sub-canvas variants (WidgetCanvas + canvas URL) — the host-plane
        // composite signature (vs an in-scene widget toggle).
        let ops = vec![
            variable_op(3, "StarMapData/CommonData/IsFullScreen"),
            json!({
                "_Pointer_": "ptr:11",
                "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                "input": "_PointsTo_:ptr:3"
            }),
            boolean_field_op(2, 3),
            boolean_field_op(4, 11),
        ];
        let subcanvas = |ptr: u32, file: &str| {
            json!({
                "_Pointer_": format!("ptr:{ptr}"),
                "_Type_": "BuildingBlocks_WidgetCanvas",
                "name": format!("Variant{ptr}"),
                "instantiated": true,
                "canvas": format!("file://./{file}.json")
            })
        };
        let make_rv = |statics: Vec<serde_json::Value>| {
            json!({
                "_Type_": "BuildingBlocks_Canvas",
                "staticVariables": statics,
                "operations": ops.clone(),
                "scene": [subcanvas(2, "large"), subcanvas(4, "small")]
            })
        };
        let result = instantiated_false_widgets(&make_rv(vec![]));
        assert!(
            !result.contains(&2),
            "Large (X, unset) must stay instantiated alongside its NOT-X sibling"
        );
        assert!(
            !result.contains(&4),
            "Small (NOT X, unset) must stay instantiated"
        );

        // Control: when the toggle has an explicit value the engine picks ONE
        // mode — normal mutual exclusivity, NOT both.
        let result_set = instantiated_false_widgets(&make_rv(vec![static_var(
            "StarMapData/CommonData/IsFullScreen",
            true,
        )]));
        assert!(!result_set.contains(&2), "Large stays when IsFullScreen=true");
        assert!(
            result_set.contains(&4),
            "Small deactivated when IsFullScreen=true (resolved value → exclusive, not both)"
        );
    }

    /// Guard the narrowing: an UNSET `X` / `NOT X` toggle gating IN-SCENE widgets
    /// (NOT sub-canvas variants — e.g. the medical/target MFD's text fields) keeps
    /// its normal exclusivity (the `NOT X` side stays, the `X` side deactivates).
    /// Without the `is_subcanvas_variant` scope this regressed `ui_target_a`.
    #[test]
    fn unset_mutually_exclusive_toggle_does_not_compose_in_scene_widgets() {
        let ops = vec![
            variable_op(3, "SomeNamespace/RuntimeToggle"),
            json!({
                "_Pointer_": "ptr:11",
                "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                "input": "_PointsTo_:ptr:3"
            }),
            boolean_field_op(2, 3),
            boolean_field_op(4, 11),
        ];
        // ptr:2 / ptr:4 are plain in-scene widgets (no `canvas` URL).
        let plain = |ptr: u32, ty: &str| {
            json!({ "_Pointer_": format!("ptr:{ptr}"), "_Type_": ty, "name": format!("W{ptr}") })
        };
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "staticVariables": [],
            "operations": ops,
            "scene": [plain(2, "BuildingBlocks_WidgetTextField"), plain(4, "BuildingBlocks_WidgetTextField")]
        });
        let result = instantiated_false_widgets(&rv);
        assert!(
            result.contains(&2),
            "in-scene X-side widget keeps normal exclusivity (deactivated), not composited"
        );
        assert!(!result.contains(&4), "in-scene NOT-X side stays (unset → true)");
    }

    /// Guard the §10 medical hazard: a `.`-grouped state variable (a multi-member
    /// state group — e.g. the medical bed's `Bed/state.BaseScreens.{Attract,
    /// MainMenu,…}`) is NOT a standalone toggle, even when one member is gated `X`
    /// and a framing widget `NOT X` over sub-canvases. The grouped cold-default
    /// mechanism picks ONE branch (MainMenu); the others stay hidden — they must
    /// NOT be composited. Without this scope, `AttractCanvas` regressed
    /// `ui_target_a` (+1 draw-order).
    #[test]
    fn grouped_state_variable_pair_does_not_compose() {
        let ops = vec![
            variable_op(3, "Bed/state.BaseScreens.Attract"),
            variable_op(5, "Bed/state.BaseScreens.MainMenu"),
            json!({
                "_Pointer_": "ptr:11",
                "_Type_": "BuildingBlocks_BindingsBooleanInvert",
                "input": "_PointsTo_:ptr:3"
            }),
            boolean_field_op(2, 3),  // AttractCanvas   = Attract
            boolean_field_op(4, 11), // Header          = NOT Attract
            boolean_field_op(6, 5),  // MainMenuCanvas  = MainMenu
        ];
        let subcanvas = |ptr: u32| {
            json!({
                "_Pointer_": format!("ptr:{ptr}"),
                "_Type_": "BuildingBlocks_WidgetCanvas",
                "name": format!("C{ptr}"),
                "canvas": format!("file://./c{ptr}.json")
            })
        };
        let rv = json!({
            "_Type_": "BuildingBlocks_Canvas",
            "staticVariables": [],
            "operations": ops,
            "scene": [subcanvas(2), subcanvas(4), subcanvas(6)]
        });
        let result = instantiated_false_widgets(&rv);
        // Cold-default picks MainMenu → MainMenuCanvas (ptr:6) shown; AttractCanvas
        // (ptr:2) stays hidden DESPITE the X / NOT-X sub-canvas pair.
        assert!(
            result.contains(&2),
            "grouped-state AttractCanvas must stay hidden (cold-default picks MainMenu), not composed"
        );
        assert!(!result.contains(&6), "grouped-state cold-default MainMenuCanvas shown");
    }

    /// The cockpit countermeasure firing overlay (`text_CountermeasureFireAmount`,
    /// the stray "0") is gated `IsActive = Or(CurrentBurstSize > 1,
    /// BurstSizeHoldRatio > 0)` — a transient firing state. At static rest both
    /// are 0 (not firing), so the overlay must hide. These are runtime
    /// `IntegerVariable` / `NumberVariable` bindings; their at-rest cold default
    /// (0) is resolved from the registry, so the `BooleanFromInteger` /
    /// `BooleanFromNumber` ordered comparisons evaluate statically and the gate
    /// resolves `false`.
    #[test]
    fn firing_overlay_hidden_when_burst_state_pinned_zero() {
        let rv = make_record_value(
            vec![],
            vec![
                json!({"_Pointer_":"ptr:13","_Type_":"BuildingBlocks_BindingsIntegerVariable","binding":"CurrentBurstSize"}),
                json!({"_Pointer_":"ptr:12","_Type_":"BuildingBlocks_BindingsNumberVariable","binding":"BurstSizeHoldRatio"}),
                json!({"_Pointer_":"ptr:19","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Greater","inputL":"_PointsTo_:ptr:13","value":1}),
                json!({"_Pointer_":"ptr:20","_Type_":"BuildingBlocks_BindingsBooleanFromNumber","type":"Greater","number":0.0,"input":"_PointsTo_:ptr:12"}),
                json!({"_Pointer_":"ptr:18","_Type_":"BuildingBlocks_BindingsBooleanEvaluateOr","inputs":["_PointsTo_:ptr:19","_PointsTo_:ptr:20"]}),
                json!({"_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:5","field":"IsActive","input":"_PointsTo_:ptr:18"}),
            ],
        );
        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path("CurrentBurstSize", crate::canvas::Value::Int(0));
        defaults.insert_path("BurstSizeHoldRatio", crate::canvas::Value::Float(0.0));
        let false_set = instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
            &rv,
            &[],
            &std::collections::HashMap::new(),
            Some(&defaults),
        );
        assert!(
            false_set.contains(&5),
            "firing overlay must hide at rest (CurrentBurstSize=0 → 0>1 false, BurstSizeHoldRatio=0 → 0>0 false)"
        );
    }

    /// Inverse guard: while actively bursting (`CurrentBurstSize` = 3) the overlay
    /// shows. Confirms the resolver computes the REAL comparison from the pinned
    /// value, not a blanket hide of every ordered-comparison gate.
    #[test]
    fn firing_overlay_shown_when_bursting() {
        let rv = make_record_value(
            vec![],
            vec![
                json!({"_Pointer_":"ptr:13","_Type_":"BuildingBlocks_BindingsIntegerVariable","binding":"CurrentBurstSize"}),
                json!({"_Pointer_":"ptr:19","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Greater","inputL":"_PointsTo_:ptr:13","value":1}),
                json!({"_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:5","field":"IsActive","input":"_PointsTo_:ptr:19"}),
            ],
        );
        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path("CurrentBurstSize", crate::canvas::Value::Int(3));
        let false_set = instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
            &rv,
            &[],
            &std::collections::HashMap::new(),
            Some(&defaults),
        );
        assert!(
            !false_set.contains(&5),
            "overlay shows while bursting (3 > 1 = true)"
        );
    }

    /// The at-rest numeric resolver is COMPONENT-LOCAL: it resolves BARE
    /// component-relative bindings only. Absolute engine-state paths (slash-
    /// prefixed) are excluded and stay on the heuristic — resolving them
    /// regressed the clipper_self_master gold baseline (ledger 88). This pins
    /// the `binding.contains('/')` discriminator at mod.rs:937: it must FAIL
    /// if that clause is deleted.
    #[test]
    fn slash_path_numeric_gate_excluded_from_static_resolution() {
        let rv = make_record_value(
            vec![],
            vec![
                // Bare component-local var → resolves at rest.
                json!({"_Pointer_":"ptr:13","_Type_":"BuildingBlocks_BindingsIntegerVariable","binding":"CurrentBurstSize"}),
                json!({"_Pointer_":"ptr:19","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Greater","inputL":"_PointsTo_:ptr:13","value":1}),
                json!({"_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:5","field":"IsActive","input":"_PointsTo_:ptr:19"}),
                // Absolute engine-state path → MUST stay on the heuristic.
                json!({"_Pointer_":"ptr:14","_Type_":"BuildingBlocks_BindingsIntegerVariable","binding":"/seatdashboard/powerstate"}),
                json!({"_Pointer_":"ptr:21","_Type_":"BuildingBlocks_BindingsBooleanFromInteger","type":"Greater","inputL":"_PointsTo_:ptr:14","value":1}),
                json!({"_Type_":"BuildingBlocks_BindingsBooleanField","widget":"_PointsTo_:ptr:6","field":"IsActive","input":"_PointsTo_:ptr:21"}),
            ],
        );
        let mut defaults = crate::defaults::DefaultValueRegistry::new();
        defaults.insert_path("CurrentBurstSize", crate::canvas::Value::Int(0));
        defaults.insert_path("/seatdashboard/powerstate", crate::canvas::Value::Int(0));
        let false_set = instantiated_false_widgets_with_param_inputs_inherited_bindings_and_defaults(
            &rv,
            &[],
            &std::collections::HashMap::new(),
            Some(&defaults),
        );
        // Bare gate resolves 0 > 1 = false → overlay 5 hides (resolver active).
        assert!(
            false_set.contains(&5),
            "bare component-local gate resolves at rest (CurrentBurstSize=0 → 0>1 false → hidden)"
        );
        // Slash-path gate stays on the heuristic → overlay 6 shown (NOT hidden).
        // FAILS if mod.rs:937's `binding.contains('/')` clause is deleted: the
        // slash var would then resolve from the registry (0 > 1 = false) and
        // wrongly hide overlay 6, regressing the frozen at-rest visibility.
        assert!(
            !false_set.contains(&6),
            "slash-path gate must stay on the heuristic (component-local scoping is load-bearing)"
        );
    }

    /// Two sub-full TILING WidgetCanvas slots (the LR-indicator master's
    /// left/right half-width columns, `width = 0.5 Percent`) form a co-rendered
    /// panel group: the screen is ONE display split by cockpit geometry. When one
    /// column is deactivated by its at-rest `Instantiated` gate (the left column's
    /// dashboard-power gate resolving false), it is KEPT instantiated alongside
    /// its always-on tiling sibling. Full-size overlay slots (MC_S_Self_Master's
    /// view modes, `Percent >= 1.0`) are NOT a tiling group and keep their
    /// exclusivity — the gated slot deactivates.
    #[test]
    fn subfull_tiling_canvas_sibling_keeps_gated_slot_instantiated() {
        // ptr:2 left column gated `Instantiated = dashboardpowered` (false at
        // rest); ptr:4 right column has no gate (always on).
        let ops = vec![
            variable_op(3, "seatdashboard/dashboardpowered"),
            boolean_field_op(2, 3),
        ];
        // Two columns anchored to opposite edges → disjoint horizontal spans.
        let slot = |ptr: u32, width: f64, anchor_x: f64| {
            json!({
                "_Pointer_": format!("ptr:{ptr}"),
                "_Type_": "BuildingBlocks_WidgetCanvas",
                "name": format!("canvas_col_{ptr}"),
                "instantiated": true,
                "parent": null,
                "anchor": {"_Type_": "Vec3", "x": anchor_x, "y": 0.0, "z": 0.0},
                "sizing": {"_Type_": "BuildingBlocks_Size",
                    "width": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": width, "behavior": "Percent"},
                    "height": {"_Type_": "BuildingBlocks_FixedOrRelativeValue", "value": 1.0, "behavior": "Percent"}}
            })
        };
        let make_rv = |width: f64| {
            json!({
                "_Type_": "BuildingBlocks_Canvas",
                "staticVariables": [static_var("seatdashboard/dashboardpowered", false)],
                "operations": ops.clone(),
                "scene": [slot(2, width, 0.0), slot(4, width, 1.0)]
            })
        };
        // TILING (half-width, opposite edges → disjoint): the gated left column
        // is kept beside its always-on sibling.
        let tiling = instantiated_false_widgets(&make_rv(0.5));
        assert!(
            !tiling.contains(&2),
            "gated tiling column must stay instantiated beside its always-on sibling"
        );
        assert!(!tiling.contains(&4), "always-on tiling column stays");
        // FULL-WIDTH (overlapping, e.g. mutually-exclusive state screens): no
        // disjoint tiling group → the gated slot deactivates.
        let overlay = instantiated_false_widgets(&make_rv(1.0));
        assert!(
            overlay.contains(&2),
            "full-width overlapping slot deactivates on a false gate (mutual exclusivity preserved)"
        );
    }
