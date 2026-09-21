//! tauri-build 装配。
//!
//! 「代码生成图标、二进制资产不入库」的落地：
//! 1. Windows 下 tauri-build 强制要求一个 `.ico` 生成可执行资源文件
//!    （缺失时报 "icons/icon.ico not found; required for generating a
//!    Windows Resource file"）——在构建期把 32×32 单色圆点 `.ico` 写进
//!    `OUT_DIR`，经 `WindowsAttributes::window_icon_path` 指给它。
//! 2. `tauri::generate_context!()` 在 bundle 未列图标时会回退加载默认的
//!    `icons/icon.png`——构建期把同款圆点写为最小手工 PNG
//!    （zlib stored 块，无压缩），`src-tauri/icons/` 整体 gitignore。

use std::env;
use std::path::PathBuf;

fn main() {
    write_if_changed(&generated_png_path(), &generated_png());
    let ico_path = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR 未设置"))
        .join("generated-icon.ico");
    std::fs::write(&ico_path, generated_ico()).expect("写入生成图标失败");
    tauri_build::try_build(
        tauri_build::Attributes::new().windows_attributes(
            tauri_build::WindowsAttributes::new().window_icon_path(ico_path),
        ),
    )
    .expect("tauri_build 失败");
}

fn generated_png_path() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 未设置"))
        .join("icons")
        .join("icon.png")
}

fn write_if_changed(path: &PathBuf, bytes: &[u8]) {
    if std::fs::read(path).map(|old| old == bytes).unwrap_or(false) {
        return;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("创建图标目录失败");
    }
    std::fs::write(path, bytes).expect("写入生成图标失败");
}

/// 圆点判定：32×32、半径 13、居中，灰 `#6b7280`。
fn led_pixel(x: u32, y: u32) -> [u8; 4] {
    const R2: f32 = 13.0 * 13.0;
    let c = 15.5f32;
    let dx = x as f32 - c;
    let dy = y as f32 - c;
    if dx * dx + dy * dy <= R2 {
        [0x6b, 0x72, 0x80, 0xff]
    } else {
        [0, 0, 0, 0]
    }
}

/// 32×32 RGBA PNG（zlib stored 块，无压缩；仅作 codegen 默认窗口图标）。
/// 用 `png` crate 正规编码（编码/解码同源）。手搓 stored-zlib 流曾因
/// 最后一块不置 BFINAL 导致 tauri-codegen 的 next_row() 静默吞错、嵌入
/// 空 RGBA 图标，运行时 tao 校验 panic（2026-09-21 验收首跑实锤）。
fn generated_png() -> Vec<u8> {
    const S: u32 = 32;
    let mut rgba = Vec::with_capacity((S * S * 4) as usize);
    for y in 0..S {
        for x in 0..S {
            rgba.extend_from_slice(&led_pixel(x, y));
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, S, S);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(&rgba).expect("png image data");
    }
    out
}

/// 32×32 灰色圆点 `.ico`（BGRA、自底向上、全零 AND 掩码）。
fn generated_ico() -> Vec<u8> {
    const S: u32 = 32;
    let mut px = Vec::with_capacity((S * S * 4) as usize);
    for y in (0..S).rev() {
        for x in 0..S {
            let [r, g, b, a] = led_pixel(x, y);
            px.extend_from_slice(&[b, g, r, a]);
        }
    }
    let and_mask = vec![0u8; (S / 8 * S) as usize]; // 32bpp 走 alpha，掩码全零
    let data_len = (40 + px.len() + and_mask.len()) as u32;

    let mut out = Vec::with_capacity(22 + data_len as usize);
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&1u16.to_le_bytes()); // count
    out.push(S as u8); // width
    out.push(S as u8); // height
    out.push(0); // palette
    out.push(0); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes()); // data offset

    out.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
    out.extend_from_slice(&(S as i32).to_le_bytes());
    out.extend_from_slice(&((S as i32) * 2).to_le_bytes()); // XOR+AND
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // 像素每米
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // 调色板
    out.extend_from_slice(&0u32.to_le_bytes()); // 重要色
    out.extend_from_slice(&px);
    out.extend_from_slice(&and_mask);
    out
}
