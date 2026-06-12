//! Loaded-PRX registry tests -- path-stem extraction and lookup by path or kernel id.

use super::*;

// -- extract_stem --

#[test]
fn extract_stem_strips_directory_and_sprx() {
    assert_eq!(
        extract_stem("/dev_flash/sys/external/libaudio.sprx"),
        "libaudio"
    );
    assert_eq!(extract_stem("external/libaudio.sprx"), "libaudio");
    assert_eq!(extract_stem("libaudio.sprx"), "libaudio");
    assert_eq!(extract_stem("libaudio.prx"), "libaudio");
    assert_eq!(extract_stem("libaudio"), "libaudio");
}

#[test]
fn extract_stem_handles_windows_separators() {
    assert_eq!(extract_stem("D:\\foo\\bar\\libaudio.sprx"), "libaudio");
}

#[test]
fn extract_stem_empty_inputs_return_empty() {
    assert_eq!(extract_stem(""), "");
    assert_eq!(extract_stem(".sprx"), "");
    assert_eq!(extract_stem(".prx"), "");
    assert_eq!(extract_stem("/"), "");
    assert_eq!(extract_stem("foo/"), "");
}

#[test]
fn extract_stem_unrecognised_extension_passes_through() {
    assert_eq!(extract_stem("libfoo.bar"), "libfoo.bar");
}

#[test]
fn extract_stem_is_case_sensitive() {
    assert_eq!(extract_stem("libaudio.SPRX"), "libaudio.SPRX");
    assert_eq!(extract_stem("libaudio.Prx"), "libaudio.Prx");
}

// -- register / lookup happy path --

fn register_libaudio(reg: &mut LoadedPrxRegistry) -> u32 {
    reg.register(
        "libaudio".to_string(),
        "cellAudio_Library".to_string(),
        0x0147_0000,
        0x0148_0000,
        0x0147_da30,
        Some(0x0147_1000),
        Some(0x0147_2000),
    )
}

#[test]
fn register_then_lookup_by_path_finds_entry() {
    let mut reg = LoadedPrxRegistry::new();
    let id = register_libaudio(&mut reg);
    let entry = reg
        .lookup_by_path("external/libaudio.sprx")
        .expect("libaudio resolves");
    assert_eq!(entry.kernel_id(), id);
    assert_eq!(entry.name(), "cellAudio_Library");
    assert_eq!(entry.base(), 0x0147_0000);
    assert_eq!(
        reg.lookup_by_path("/dev_flash/sys/external/libaudio.sprx")
            .unwrap()
            .kernel_id(),
        id
    );
}

#[test]
fn lookup_by_unknown_path_returns_none() {
    let mut reg = LoadedPrxRegistry::new();
    register_libaudio(&mut reg);
    assert!(reg.lookup_by_path("external/libfoo.sprx").is_none());
}

#[test]
fn lookup_by_empty_path_returns_none() {
    let mut reg = LoadedPrxRegistry::new();
    register_libaudio(&mut reg);
    assert!(reg.lookup_by_path("").is_none());
    assert!(reg.lookup_by_path(".sprx").is_none());
    assert!(reg.lookup_by_path("foo/").is_none());
}

#[test]
fn lookup_by_id_returns_registered_entry() {
    let mut reg = LoadedPrxRegistry::new();
    let id = register_libaudio(&mut reg);
    assert!(reg.lookup_by_id(id).is_some());
    assert!(reg.lookup_by_id(id + 1).is_none());
}

#[test]
fn lookup_by_id_and_path_return_same_entry() {
    let mut reg = LoadedPrxRegistry::new();
    let id = register_libaudio(&mut reg);
    let by_id = reg.lookup_by_id(id).unwrap();
    let by_path = reg.lookup_by_path("libaudio.sprx").unwrap();
    assert!(std::ptr::eq(by_id, by_path));
}

#[test]
fn round_trip_preserves_every_field() {
    let mut reg = LoadedPrxRegistry::new();
    let id = reg.register(
        "libsentinel".to_string(),
        "X_Y_Z".to_string(),
        0x1111_0000,
        0x1112_0000,
        0x1111_d000,
        Some(0x1111_1000),
        Some(0x1111_2000),
    );
    let entry = reg.lookup_by_id(id).unwrap();
    assert_eq!(entry.kernel_id(), id);
    assert_eq!(entry.stem(), "libsentinel");
    assert_eq!(entry.name(), "X_Y_Z");
    assert_eq!(entry.base(), 0x1111_0000);
    assert_eq!(entry.data_end(), 0x1112_0000);
    assert_eq!(entry.toc(), 0x1111_d000);
    assert_eq!(entry.start_opd(), Some(0x1111_1000));
    assert_eq!(entry.stop_opd(), Some(0x1111_2000));
}

// -- ids / counters --

#[test]
fn first_kernel_id_is_named_constant() {
    let mut reg = LoadedPrxRegistry::new();
    let id = register_libaudio(&mut reg);
    assert_eq!(id, FIRST_KERNEL_ID);
}

#[test]
fn consecutive_ids_increment_by_one() {
    let mut reg = LoadedPrxRegistry::new();
    let a = reg.register("liba".into(), "A".into(), 0, 0, 0, None, None);
    let b = reg.register("libb".into(), "B".into(), 0, 0, 0, None, None);
    let c = reg.register("libc".into(), "C".into(), 0, 0, 0, None, None);
    assert_eq!(b, a + 1);
    assert_eq!(c, a + 2);
}

#[test]
fn is_empty_tracks_registration() {
    let mut reg = LoadedPrxRegistry::new();
    assert!(reg.is_empty());
    register_libaudio(&mut reg);
    assert!(!reg.is_empty());
}

#[test]
fn ids_on_empty_registry_yields_nothing() {
    let reg = LoadedPrxRegistry::new();
    assert_eq!(reg.ids().count(), 0);
}

#[test]
fn ids_iterates_in_monotonic_order() {
    let mut reg = LoadedPrxRegistry::new();
    reg.register("liba".into(), "A".into(), 0, 0, 0, None, None);
    reg.register("libb".into(), "B".into(), 0, 0, 0, None, None);
    reg.register("libc".into(), "C".into(), 0, 0, 0, None, None);
    let ids: Vec<u32> = reg.ids().collect();
    assert_eq!(ids.len(), 3);
    assert!(ids[0] < ids[1]);
    assert!(ids[1] < ids[2]);
}

// -- duplicate registration --

#[test]
#[should_panic(expected = "already registered")]
fn register_same_stem_twice_panics() {
    let mut reg = LoadedPrxRegistry::new();
    reg.register("libaudio".into(), "A".into(), 0, 0, 0, None, None);
    reg.register("libaudio".into(), "B".into(), 0, 0, 0, None, None);
}

// -- precondition guards (debug-only) --

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "non-empty")]
fn register_empty_stem_panics_debug() {
    let mut reg = LoadedPrxRegistry::new();
    reg.register(String::new(), "A".into(), 0, 0, 0, None, None);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "not already normalised")]
fn register_unstripped_extension_panics_debug() {
    let mut reg = LoadedPrxRegistry::new();
    reg.register("libaudio.sprx".into(), "A".into(), 0, 0, 0, None, None);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "not already normalised")]
fn register_dir_prefix_panics_debug() {
    let mut reg = LoadedPrxRegistry::new();
    reg.register("foo/bar".into(), "A".into(), 0, 0, 0, None, None);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "data_end")]
fn register_data_end_below_base_panics_debug() {
    let mut reg = LoadedPrxRegistry::new();
    reg.register(
        "libaudio".into(),
        "A".into(),
        0x1000_0000,
        0x0FFF_F000,
        0x1000_8000,
        None,
        None,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "start_opd")]
fn register_start_opd_outside_segment_panics_debug() {
    let mut reg = LoadedPrxRegistry::new();
    reg.register(
        "libaudio".into(),
        "A".into(),
        0x1000_0000,
        0x1010_0000,
        0x1000_8000,
        Some(0x2000_0000),
        None,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "stop_opd")]
fn register_stop_opd_outside_segment_panics_debug() {
    let mut reg = LoadedPrxRegistry::new();
    reg.register(
        "libaudio".into(),
        "A".into(),
        0x1000_0000,
        0x1010_0000,
        0x1000_8000,
        None,
        Some(0x2000_0000),
    );
}
