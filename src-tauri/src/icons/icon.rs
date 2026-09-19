//! 代码生成的三态托盘图标（32×32 RGBA），二进制资产不入库。
//!
//! 形状：透明背景上的居中实心圆点（状态灯）。颜色取 brief 指定值：
//! 灰 `#6b7280` / 绿 `#10b981` / 琥珀 `#f59e0b`。

use tauri::image::Image;

const SIZE: u32 = 32;
/// 圆点半径（像素），留 3px 透明边。
const RADIUS: i32 = 13;

/// 灰：未连接。
pub fn grey_icon() -> Image<'static> {
    led(0x6b, 0x72, 0x80)
}

/// 绿：已连接。
pub fn green_icon() -> Image<'static> {
    led(0x10, 0xb9, 0x81)
}

/// 琥珀：有待确认。
pub fn amber_icon() -> Image<'static> {
    led(0xf5, 0x9e, 0x0b)
}

fn led(r: u8, g: u8, b: u8) -> Image<'static> {
    let half = (SIZE / 2) as i32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let inside = (x - half) * (x - half) + (y - half) * (y - half) <= RADIUS * RADIUS;
            if inside {
                rgba.extend_from_slice(&[r, g, b, 0xff]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Image::new_owned(rgba, SIZE, SIZE)
}
