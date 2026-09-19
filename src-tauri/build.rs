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
fn generated_png() -> Vec<u8> {
    const S: u32 = 32;
    let mut raw = Vec::with_capacity((S * (1 + S * 4)) as usize);
    for y in 0..S {
        raw.push(0); // filter: None
        for x in 0..S {
            raw.extend_from_slice(&led_pixel(x, y));
        }
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&S.to_be_bytes());
    ihdr.extend_from_slice(&S.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8bit RGBA
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    push_chunk(&mut out, b"IEND", &[]);
    out
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(tag);
    crc_input.extend_from_slice(data);
    let crc = crc32(&crc_input);
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// zlib 流：0x78 0x01 头 + stored deflate 块 + adler32。
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    if raw.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    }
    for chunk in raw.chunks(65_535) {
        out.push(u8::from(chunk.len() == 65_535));
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
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
