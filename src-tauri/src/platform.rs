//! 平台窗体材质决策（2026-09-23 设计，用户批准）：
//! Win11(build≥22000)=Mica，Win10=Acrylic；macOS=UnderWindowBackground 振效，
//! 圆角随版本：Golden Gate(27+)=20、Tahoe(26)=26、更早=10；Windows 一律 8（CSS 自绘）。
//! 决策函数为纯函数（可单测），detect() 用 os_info 做运行时探测。

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformChrome {
    WindowsMica,
    WindowsAcrylic,
    MacOS { radius: u32 },
    Other { radius: u32 },
}

impl PlatformChrome {
    /// webview 启动补拉载荷：os 类型 + CSS 圆角值（px）。
    pub fn as_json(&self) -> Value {
        let (os, radius) = match self {
            Self::WindowsMica | Self::WindowsAcrylic => ("windows", 8u32),
            Self::MacOS { radius } => ("macos", *radius),
            Self::Other { radius } => ("other", *radius),
        };
        json!({ "os": os, "radius": radius })
    }
}

/// 纯函数：Windows build 号 → 材质。22000 起为 Win11。
pub fn chrome_for_windows_build(build: u32) -> PlatformChrome {
    if build >= 22000 {
        PlatformChrome::WindowsMica
    } else {
        PlatformChrome::WindowsAcrylic
    }
}

/// 纯函数：macOS 主版本号 → 圆角（Golden Gate 统一 20pt，Tahoe 26pt，更早 10pt）。
pub fn chrome_for_macos_major(major: u64) -> PlatformChrome {
    let radius = if major >= 27 {
        20
    } else if major == 26 {
        26
    } else {
        10
    };
    PlatformChrome::MacOS { radius }
}

/// 运行时探测。os_info 的 Version：Windows=Semantic(major, minor, build)，
/// macOS=ProductVersion 的 Semantic(major, minor, patch)。
pub fn detect() -> PlatformChrome {
    let info = os_info::get();
    let (major, patch) = match info.version() {
        os_info::Version::Semantic(major, _, patch) => (*major, *patch as u32),
        _ => (0, 0),
    };
    match info.os_type() {
        os_info::Type::Windows => chrome_for_windows_build(patch),
        os_info::Type::Macos => chrome_for_macos_major(major),
        other => {
            let _ = other;
            PlatformChrome::Other { radius: 10 }
        }
    }
}
