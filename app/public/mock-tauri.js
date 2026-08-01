// TEMPORARY design-capture mock of the Tauri IPC bridge. Serves canned data so
// the frontend runs in a plain browser for screenshots. Not part of the app.
(function () {
  "use strict";

  // ── canned data ──────────────────────────────────────────────────────────
  const ships = [
    ["AEGS_Gladius", "Aegis Gladius"],
    ["AEGS_Sabre", "Aegis Sabre"],
    ["ANVL_Arrow", "Anvil Arrow"],
    ["ANVL_C8X_Pisces_Expedition", "Anvil C8X Pisces Expedition"],
    ["ANVL_Carrack", "Anvil Carrack"],
    ["CNOU_Mustang_Alpha", "C.O. Mustang Alpha"],
    ["CRUS_Star_Runner", "Crusader Mercury Star Runner"],
    ["DRAK_Clipper", "Drake Clipper"],
    ["DRAK_Cutlass_Black", "Drake Cutlass Black"],
    ["DRAK_Caterpillar", "Drake Caterpillar"],
    ["MISC_Freelancer", "MISC Freelancer"],
    ["MISC_Prospector", "MISC Prospector"],
    ["ORIG_300i", "Origin 300i"],
    ["ORIG_890Jump", "Origin 890 Jump"],
    ["RSI_Aurora_MR", "RSI Aurora MR"],
    ["RSI_Constellation_Andromeda", "RSI Constellation Andromeda"],
    ["AEGS_Idris_P", "Aegis Idris-P"],
    ["AEGS_Reclaimer", "Aegis Reclaimer"],
    ["AEGS_Gladius_Pirate", "Aegis Gladius Pirate", true],
    ["DRAK_Cutlass_Black_AI_CrimStat", "Drake Cutlass Black (AI)", true],
  ];
  const vehicles = [
    ["ANVL_Atlas_Geo", "Anvil ATLS GEO"],
    ["GRIN_PTV", "Greycat PTV"],
    ["GRIN_ROC", "Greycat ROC"],
    ["TMBL_Cyclone", "Tumbril Cyclone"],
    ["TMBL_Nova", "Tumbril Nova"],
    ["TMBL_Storm", "Tumbril Storm"],
  ];
  const weapons = [
    ["BEHR_LaserRepeater_S3", "Behring M5A Laser Repeater"],
    ["GATS_BallisticGatling_S3", "Gallenson Tarantula GT-870"],
    ["KLWE_LaserRepeater_S2", "Klaus & Werner CF-227"],
    ["APAR_BallisticGatling_S4", "Apocalypse Arms Revenant"],
  ];
  const mkEnt = ([name, display, npc]) => ({
    name, id: "guid-" + name.toLowerCase(),
    display_name: display, is_npc_or_internal: !!npc,
  });

  const socpaks = [
    ["Data/ObjectContainers/PU/loc/stanton/area18/area18.socpak", "Cities & Landing Zones", "Stanton"],
    ["Data/ObjectContainers/PU/loc/stanton/lorville/lorville.socpak", "Cities & Landing Zones", "Stanton"],
    ["Data/ObjectContainers/PU/loc/stanton/newbab/newbab.socpak", "Cities & Landing Zones", "Stanton"],
    ["Data/ObjectContainers/PU/loc/pyro/ruinstation/ruinstation.socpak", "Space Stations", "Pyro"],
    ["Data/ObjectContainers/PU/loc/stanton/porttressler/porttressler.socpak", "Space Stations", "Stanton"],
    ["Data/ObjectContainers/PU/loc/stanton/everusharbor/everusharbor.socpak", "Space Stations", "Stanton"],
    ["Data/ObjectContainers/PU/loc/stanton/rnr_stanton_1/rest_stop_a.socpak", "Space Stations", "Rest Stops"],
    ["Data/ObjectContainers/PU/loc/outposts/shubin_mining_smca6.socpak", "Outposts & Surface Bases", "Mining"],
    ["Data/ObjectContainers/PU/loc/outposts/hdms_oparei.socpak", "Outposts & Surface Bases", "HDMS"],
    ["Data/ObjectContainers/PU/loc/caves/cave_sand_medium_01.socpak", "Underground & Caves", "Sand"],
    ["Data/ObjectContainers/PU/loc/derelicts/drak_caterpillar_wreck_01.socpak", "Derelicts & Wrecks", "Ships"],
    ["Data/ObjectContainers/hangars/hangar_selfland_medium.socpak", "Hangars", "SelfLand"],
    ["Data/ObjectContainers/hangars/hangar_aeroview_large.socpak", "Hangars", "Aeroview"],
    ["Data/ObjectContainers/shops/dumpersdepot_area18.socpak", "Shops & Interiors", "Area18"],
    ["Data/ObjectContainers/shops/centermass_newbab.socpak", "Shops & Interiors", "New Babbage"],
    ["Data/ObjectContainers/ships/aegs_idris/aegs_idris_interior.socpak", "Ships", "Aegis"],
    ["Data/ObjectContainers/pu/modules/lighting_global.socpak", "Shared Modules & Lighting", "General"],
    ["Data/ObjectContainers/props/flair_collection_01.socpak", "Props, Flair & Decor", "General"],
  ].map(([path, category, subcategory]) => ({ path, category, subcategory }));

  const p4kTree = {
    "": [
      d("Data"), d("Engine"),
      f("Game.dcb", 310_432_512),
    ],
    "Data": [d("Levels"), d("Libs"), d("Objects"), d("ObjectContainers"), d("Sounds"), d("Textures"), d("UI")],
    "Data/Objects": [d("Spaceships"), d("buildingsets"), d("planets")],
    "Data/Objects/Spaceships": [d("Ships")],
    "Data/Objects/Spaceships/Ships": [d("AEGS"), d("ANVL"), d("DRAK"), d("MISC"), d("ORIG"), d("RSI")],
    "Data/Objects/Spaceships/Ships/DRAK": [
      d("Cutlass"), d("Clipper"),
      f("DRAK_Cutlass_Black.cga", 48_211_004),
      f("DRAK_Cutlass_Black.cgam", 122_306_711),
      f("DRAK_Cutlass_Black.chrparams", 12_884),
      f("DRAK_Cutlass_Black.dba", 2_493_115),
      f("DRAK_Cutlass_Black_LOD1.cga", 18_022_400),
      f("cutlass_black_exterior.mtl", 91_233),
      f("cutlass_black_interior.mtl", 78_450),
      f("cutlass_hull_diff.dds", 22_369_622),
      f("cutlass_hull_ddna.dds", 22_369_622),
    ],
  };
  function d(name) { return { kind: "directory", name }; }
  function f(name, size) {
    return { kind: "file", name, compressed_size: Math.round(size * 0.6), uncompressed_size: size };
  }
  function fallbackDir(path) {
    const base = path.split("/").pop() || "dir";
    return [d(base + "_detail"), f(base + "_01.cgf", 4_210_688), f(base + "_01.mtl", 18_320), f(base + "_diff.dds", 11_184_811)];
  }

  const dcTree = {
    "": [dcF("EntityClassDefinitions"), dcF("Items"), dcF("Loadouts"), dcF("MissionBrokers"), dcF("Ships"), dcF("TagDatabase"), dcF("Vehicles")],
    "EntityClassDefinitions": [
      dcF("Spaceships"),
      dcR("EntityClassDefinition.DRAK_Cutlass_Black"),
      dcR("EntityClassDefinition.AEGS_Gladius"),
      dcR("EntityClassDefinition.ANVL_Carrack"),
    ],
    "EntityClassDefinitions/Spaceships": [
      dcR("EntityClassDefinition.ORIG_890Jump"),
      dcR("EntityClassDefinition.RSI_Constellation_Andromeda"),
    ],
  };
  function dcF(name) { return { kind: "folder", name }; }
  function dcR(name) {
    return { kind: "record", name, struct_type: "EntityClassDefinition", id: "rec-" + name.toLowerCase() };
  }
  const dcRecordJson = JSON.stringify({
    __type: "EntityClassDefinition",
    ClassName: "DRAK_Cutlass_Black",
    Category: "@vehicle_focus_medium_fighter",
    Icon: "icon_cutlass_black",
    Components: {
      SAttachableComponentParams: {
        AttachDef: { Type: "Vehicle", SubType: "Vehicle_Spaceship", Size: 4, Grade: 1, Manufacturer: "DRAK" },
        Localization: { Name: "@vehicle_NameCutlassBlack", ShortName: "@vehicle_ShortNameCutlass" },
      },
      VehicleComponentParams: {
        vehicleDefinition: "Objects/Spaceships/Ships/DRAK/Cutlass/DRAK_Cutlass.xml",
        crewSize: 2, dogfightEnabled: true,
        maxBoundingBoxSize: { x: 29.3, y: 26.5, z: 10.2 },
      },
      SEntityComponentDefaultLoadoutParams: {
        loadout: { entries: [
          { itemPortName: "hardpoint_weapon_left_wing", entityClassName: "BEHR_LaserRepeater_S3" },
          { itemPortName: "hardpoint_weapon_right_wing", entityClassName: "BEHR_LaserRepeater_S3" },
          { itemPortName: "hardpoint_turret", entityClassName: "MRCK_S4_DRAK_Cutlass" },
        ]},
      },
    },
  });

  const banks = [
    ["amb_area18", 412], ["amb_lorville", 388], ["global_music", 951],
    ["ship_drake_cutlass", 264], ["ship_aegs_gladius", 231],
    ["weapons_ballistic", 508], ["weapons_energy", 477], ["ui_global", 189],
  ].map(([name, trigger_count]) => ({ name, trigger_count }));
  const trigDetail = (n, i) => ({
    trigger_name: n, bank_name: "ship_drake_cutlass",
    duration_type: i % 3 === 0 ? "Infinite" : "OneShot", sound_count: (i % 4) + 1,
  });
  const bankTriggers = [
    "Play_ship_drake_cutlass_engine_start", "Play_ship_drake_cutlass_engine_loop",
    "Stop_ship_drake_cutlass_engine_loop", "Play_ship_drake_cutlass_thruster_burst",
    "Play_ship_drake_cutlass_door_open", "Play_ship_drake_cutlass_door_close",
    "Play_ship_drake_cutlass_gear_deploy", "Play_ship_drake_cutlass_qd_spool",
    "Play_ship_drake_cutlass_hit_metal", "Play_ship_drake_cutlass_alarm_hull",
  ].map(trigDetail);
  const sounds = [
    [104882311, "Embedded", "engine_start_layer_a"],
    [104882312, "Embedded", "engine_start_layer_b"],
    [208113940, "Streamed", "engine_loop_core"],
    [208113941, "Streamed", "engine_loop_whine"],
  ].map(([media_id, source_type, desc]) => ({
    media_id, source_type, bank_name: "ship_drake_cutlass", path_description: desc,
  }));
  const audioEntities = [
    ["DRAK_Cutlass_Black", "EntityClassDefinitions/DRAK_Cutlass_Black", 42],
    ["AEGS_Gladius", "EntityClassDefinitions/AEGS_Gladius", 38],
    ["hangar_selfland_medium", "ObjectContainers/hangar_selfland", 17],
  ].map(([name, record_path, trigger_count]) => ({ name, record_path, trigger_count }));

  const socpakHierarchy = [{
    path: socpaks[11].path, name: "hangar_selfland_medium", entity_name: "Hangar_SelfLand_Med",
    class_name: "ObjectContainer", depth: 0, mesh_count: 148, light_count: 62,
    children: [
      { path: "child/interior_shell.socpak", name: "interior_shell", entity_name: null, class_name: "ObjectContainer", depth: 1, mesh_count: 96, light_count: 40, children: [] },
      { path: "child/props_dressing.socpak", name: "props_dressing", entity_name: null, class_name: "ObjectContainer", depth: 1, mesh_count: 52, light_count: 8,
        children: [{ path: "child/props_dressing/crates.socpak", name: "crates", entity_name: null, class_name: "ObjectContainer", depth: 2, mesh_count: 21, light_count: 0, children: [] }] },
      { path: "child/lighting_rig.socpak", name: "lighting_rig", entity_name: null, class_name: "LightGroup", depth: 1, mesh_count: 0, light_count: 14, children: [] },
    ],
  }];

  // ── event plumbing ───────────────────────────────────────────────────────
  const callbacks = new Map();   // callbackId -> fn
  const listeners = new Map();   // eventName -> Set<callbackId>
  let nextId = 1;
  window.__mockEmit = function (event, payload) {
    const set = listeners.get(event);
    if (!set) return 0;
    for (const id of set) {
      const cb = callbacks.get(id);
      if (cb) cb({ event, id, payload });
    }
    return set.size;
  };

  // ── command handlers ─────────────────────────────────────────────────────
  const commands = {
    get_app_version: () => ({ version: "0.9.4+f3ab21c" }),
    get_system_theme: () => { throw new Error("mock: keep authored theme"); },
    discover_p4k: () => ([
      { path: "C:\\Program Files\\Roberts Space Industries\\StarCitizen\\LIVE\\Data.p4k", source: "LIVE" },
      { path: "C:\\Program Files\\Roberts Space Industries\\StarCitizen\\PTU\\Data.p4k", source: "PTU" },
    ]),
    open_p4k: () => ({ entry_count: 248_113, total_bytes: 131_204_882_432 }),
    list_dir: (a) => p4kTree[a.path] ?? fallbackDir(a.path),
    list_subdirs: (a) => (p4kTree[a.path] ?? []).filter((e) => e.kind === "directory").map((e) => e.name),
    p4k_search: (a) => {
      const results = [
        "Data/Objects/Spaceships/Ships/DRAK/DRAK_Cutlass_Black.cga",
        "Data/Objects/Spaceships/Ships/DRAK/cutlass_black_exterior.mtl",
        "Data/Textures/ships/drak/cutlass_hull_diff.dds",
        "Data/Sounds/wwise/ship_drake_cutlass.bnk",
      ].filter((p) => p.toLowerCase().includes((a.query || "").toLowerCase()))
        .map((path, i) => ({ path, uncompressed_size: 22_369_622 - i * 3_000_000, modified_unix: 1_760_000_000 - i * 86_400 }));
      return { results, total: results.length };
    },
    scan_categories: () => ([
      { name: "Ships", entities: ships.map(mkEnt) },
      { name: "Vehicles", entities: vehicles.map(mkEnt) },
      { name: "Weapons", entities: weapons.map(mkEnt) },
    ]),
    list_blender_addon_targets: () => ({
      current_version: "1.4.2",
      targets: [
        { blender_version: "5.0", addons_path: "C:\\Users\\owner\\AppData\\Roaming\\Blender\\5.0\\scripts\\addons", state: "installed", installed_version: "1.4.2" },
        { blender_version: "5.1", addons_path: "C:\\Users\\owner\\AppData\\Roaming\\Blender\\5.1\\scripts\\addons", state: "upgrade", installed_version: "1.3.0" },
      ],
      blender_running: false, incompatible_blender_found: false,
    }),
    start_export: () => null,
    cancel_export: () => null,
    scan_socpaks: (a) => {
      const q = ((a && a.query) || "").toLowerCase();
      return q ? socpaks.filter((s) => s.path.toLowerCase().includes(q)) : socpaks;
    },
    inspect_socpak_hierarchy: (a) =>
      a.request.socpak_paths.length === 1 ? socpakHierarchy
        : a.request.socpak_paths.map((p, i) => ({ ...socpakHierarchy[0], path: p, name: p.split("/").pop() || String(i) })),
    export_socpaks: () => new Promise(() => {}), // stays "exporting" for the overlay shot
    dc_search: (a) => [
      "EntityClassDefinition.DRAK_Cutlass_Black", "EntityClassDefinition.DRAK_Cutlass_Red",
      "VehicleDefinition.DRAK_Cutlass", "TintPalette.drak_cutlass_black_default",
    ].filter((n) => n.toLowerCase().includes(a.query.toLowerCase()))
      .map((name, i) => ({ name, struct_type: name.split(".")[0], path: "EntityClassDefinitions/" + name, id: "rec-" + i })),
    dc_list_tree: (a) => dcTree[a.path] ?? [dcR("EntityClassDefinition.Sample_Record")],
    dc_get_record: (a) => ({
      name: "EntityClassDefinition.DRAK_Cutlass_Black", struct_type: "EntityClassDefinition",
      path: "EntityClassDefinitions/EntityClassDefinition.DRAK_Cutlass_Black",
      id: a.recordId, json: dcRecordJson,
    }),
    dc_get_backlinks: () => ([
      { name: "ShipShop.NewDeal_Lorville", id: "rec-bl-1" },
      { name: "MissionBroker.PU_Delivery", id: "rec-bl-2" },
    ]),
    audio_init: () => ({ trigger_count: 48_211, bank_count: 312 }),
    audio_list_banks: () => banks,
    audio_search_triggers: () => bankTriggers.map((t) => ({
      trigger_name: t.trigger_name, bank_name: t.bank_name,
      duration_type: t.duration_type, radius_max: t.duration_type === "Infinite" ? 120 : null,
    })),
    audio_search_entities: () => audioEntities,
    audio_bank_triggers: () => bankTriggers,
    audio_bank_media: () => sounds,
    audio_entity_triggers: () => bankTriggers.slice(0, 6),
    audio_resolve_trigger: () => sounds,
    audio_export_info: () => ({ extension: "ogg" }),

    // plugin commands
    "plugin:event|listen": (a) => {
      let set = listeners.get(a.event);
      if (!set) { set = new Set(); listeners.set(a.event, set); }
      set.add(a.handler);
      return nextId++;
    },
    "plugin:event|unlisten": () => null,
    "plugin:log|attach_console": () => nextId++,
    "plugin:dialog|open": (a) =>
      a && a.options && a.options.directory ? "D:\\StarCitizen\\exports" : "C:\\Program Files\\Roberts Space Industries\\StarCitizen\\LIVE\\Data.p4k",
  };

  window.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main", windowLabel: "main" },
    },
    transformCallback: function (cb) {
      const id = nextId++;
      callbacks.set(id, cb);
      return id;
    },
    invoke: function (cmd, args) {
      const handler = commands[cmd];
      if (!handler) return Promise.reject("mock: unhandled command " + cmd);
      try {
        return Promise.resolve(handler(args || {}));
      } catch (e) {
        return Promise.reject(String(e));
      }
    },
    plugins: {},
  };
})();
