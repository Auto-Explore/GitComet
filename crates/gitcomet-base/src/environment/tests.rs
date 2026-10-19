use super::*;

#[test]
fn summary_uses_the_same_rows_as_settings_and_preserves_partial_details() {
    let mut snapshot = EnvironmentSnapshot::default();
    snapshot.graphics.insert(
        1,
        GraphicsDetails {
            window_system: Some("X11".into()),
            device_name: Some("  GPU\nmodel  ".into()),
            driver_name: Some(" ".into()),
            rendering: Rendering::Hardware,
            ..Default::default()
        },
    );
    let summary = snapshot.summary();
    assert!(summary.contains("Selected GPU: GPU model\n"));
    assert!(summary.contains("Graphics backend: Unavailable\n"));
    assert!(summary.contains("Driver: Unavailable\n"));
    assert!(summary.contains("Rendering: Hardware\n"));
    for section in snapshot.sections() {
        for row in section.rows {
            assert!(summary.contains(&format!("{}: {}\n", row.label, row.value)));
        }
    }
}

#[test]
fn distinct_window_configurations_are_grouped_without_losing_software_rendering() {
    let mut snapshot = EnvironmentSnapshot::default();
    let hardware = GraphicsDetails {
        rendering: Rendering::Hardware,
        backend: Some("OpenGL".into()),
        ..Default::default()
    };
    snapshot.graphics.insert(1, hardware.clone());
    snapshot.graphics.insert(2, hardware);
    snapshot.graphics.insert(
        3,
        GraphicsDetails {
            rendering: Rendering::Software,
            backend: Some("Vulkan".into()),
            ..Default::default()
        },
    );
    let sections = snapshot.sections();
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[1].title, "Graphics 1 (2 windows)");
    assert_eq!(sections[2].title, "Graphics 2 (1 window)");
    let summary = snapshot.summary();
    assert!(summary.contains("Rendering: Software (CPU)"));
    assert!(summary.contains("Graphics backend: OpenGL"));
    assert!(summary.contains("Graphics backend: Vulkan"));
    assert_eq!(
        serde_json::from_str::<EnvironmentSnapshot>(&serde_json::to_string(&snapshot).unwrap())
            .unwrap(),
        snapshot
    );
}

#[test]
fn missing_gpu_is_unavailable_not_hardware() {
    let snapshot = EnvironmentSnapshot::default();
    let summary = snapshot.summary();
    assert!(summary.contains("Selected GPU: Unavailable"));
    assert!(summary.contains("Rendering: Unavailable"));
    assert!(!summary.contains("Hardware"));
}

#[test]
fn partial_recorded_snapshot_tolerates_missing_fields() {
    let snapshot: EnvironmentSnapshot = serde_json::from_str(r#"{"app_version":"0.2.5","graphics":{"1":{"device_name":"llvmpipe","rendering":"Software"}}}"#).unwrap();
    assert!(snapshot.summary().contains("Build: GitComet v0.2.5"));
    assert!(snapshot.summary().contains("Selected GPU: llvmpipe"));
    assert!(snapshot.summary().contains("Driver details: Unavailable"));
}

#[test]
fn memory_uses_bytes_and_system_collection_is_cached() {
    let mut snapshot = EnvironmentSnapshot::default();
    snapshot.system.total_memory_bytes = Some(16 * 1_073_741_824);
    assert!(snapshot.summary().contains("Total memory: 16.0 GiB"));
    assert_eq!(SystemDetails::collect_once(), SystemDetails::collect_once());
}
