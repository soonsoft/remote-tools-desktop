//! 平台窗体材质决策的纯函数测试（TDD 先行——实现见 src/platform.rs）。
use remote_tools_desktop::platform::{
    chrome_for_macos_major, chrome_for_windows_build, PlatformChrome,
};

#[test]
fn windows_build_22000_and_up_is_mica() {
    // Win11 各版本：22000（21H2）… 26100（24H2）
    for build in [22000u32, 22631, 26100, 30000] {
        assert!(matches!(
            chrome_for_windows_build(build),
            PlatformChrome::WindowsMica
        ));
    }
}

#[test]
fn windows_build_below_22000_is_acrylic() {
    // Win10：10240（1507）… 19045（22H2）
    for build in [10240u32, 17763, 19045, 21999] {
        assert!(matches!(
            chrome_for_windows_build(build),
            PlatformChrome::WindowsAcrylic
        ));
    }
}

#[test]
fn macos_27_golden_gate_is_unified_20() {
    match chrome_for_macos_major(27) {
        PlatformChrome::MacOS { radius } => assert_eq!(radius, 20),
        other => panic!("expected MacOS, got {other:?}"),
    }
}

#[test]
fn macos_26_tahoe_is_26() {
    match chrome_for_macos_major(26) {
        PlatformChrome::MacOS { radius } => assert_eq!(radius, 26),
        other => panic!("expected MacOS, got {other:?}"),
    }
}

#[test]
fn macos_15_and_older_is_10() {
    for major in [11u64, 12, 13, 14, 15] {
        match chrome_for_macos_major(major) {
            PlatformChrome::MacOS { radius } => assert_eq!(radius, 10, "major {major}"),
            other => panic!("expected MacOS, got {other:?}"),
        }
    }
}

#[test]
fn chrome_json_carries_os_and_radius() {
    assert_eq!(
        chrome_for_windows_build(26100).as_json(),
        serde_json::json!({ "os": "windows", "radius": 8 })
    );
    assert_eq!(
        chrome_for_macos_major(27).as_json(),
        serde_json::json!({ "os": "macos", "radius": 20 })
    );
}
