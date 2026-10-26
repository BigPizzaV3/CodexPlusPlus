// Tauri 在 Windows 上只取 icon.ico 的第一个条目作为窗口和托盘图标；
// 第一个条目是 16×16 时任务栏和托盘图标会被放大发糊（#2662）。
#[test]
fn windows_icon_lists_largest_entry_first() {
    let bytes = include_bytes!("../icons/icon.ico");
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    assert!(count > 0, "icon.ico 没有任何条目");
    // ICO 目录项宽度字节为 0 表示 256
    let width = |index: usize| match bytes[6 + 16 * index] {
        0 => 256,
        value => value as u32,
    };
    let largest = (0..count).map(width).max().unwrap();
    assert_eq!(width(0), largest, "icon.ico 第一个条目必须是最大尺寸");
    assert_eq!(largest, 256);
}
