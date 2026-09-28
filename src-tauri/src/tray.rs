use crate::scanner::SystemHealth;
use std::sync::Mutex;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

/// 持有菜单项引用，供 monitor 线程定期更新文字
pub struct TrayItems {
    pub cpu: Mutex<MenuItem<tauri::Wry>>,
    pub mem: Mutex<MenuItem<tauri::Wry>>,
    pub disk: Mutex<MenuItem<tauri::Wry>>,
    pub health_header: Mutex<MenuItem<tauri::Wry>>,
}

pub fn init_tray(app: &AppHandle) -> tauri::Result<()> {
    // 动态状态区（只读项，靠 monitor 线程刷新）
    let health_header = MenuItem::with_id(app, "health_header", "系统状态", false, None::<&str>)?;
    let cpu_item = MenuItem::with_id(app, "cpu_item", "  CPU:    —", false, None::<&str>)?;
    let mem_item = MenuItem::with_id(app, "mem_item", "  内存:   —", false, None::<&str>)?;
    let disk_item = MenuItem::with_id(app, "disk_item", "  磁盘:   —", false, None::<&str>)?;

    // 操作区
    let open_item = MenuItem::with_id(app, "open", "打开 MacSlim", true, None::<&str>)?;
    let scan_item = MenuItem::with_id(app, "scan", "立即扫描", true, None::<&str>)?;
    let optimize_item =
        MenuItem::with_id(app, "optimize", "一键优化（安全项）", true, None::<&str>)?;

    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let about_item = MenuItem::with_id(app, "about", "关于 MacSlim v0.1.0", false, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 MacSlim", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &health_header,
            &cpu_item,
            &mem_item,
            &disk_item,
            &sep1,
            &open_item,
            &scan_item,
            &optimize_item,
            &sep2,
            &about_item,
            &quit_item,
        ],
    )?;

    // 存入 app state 供 monitor 更新
    app.manage(TrayItems {
        cpu: Mutex::new(cpu_item),
        mem: Mutex::new(mem_item),
        disk: Mutex::new(disk_item),
        health_header: Mutex::new(health_header),
    });

    // 加载菜单栏专用模板图标（纯黑 sparkle 轮廓，macOS 自动适配深浅色）
    let tray_icon = load_tray_icon(app);

    let _ = TrayIconBuilder::with_id("main-tray")
        .icon(tray_icon)
        .icon_as_template(true)
        .tooltip("MacSlim")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            // 左键点击 → 切换窗口显隐（再次点击会把窗口收回）
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                show_and_focus(app);
            }
            "scan" => {
                show_and_focus(app);
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("tray:scan", ());
                }
            }
            "optimize" => {
                show_and_focus(app);
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("tray:optimize", ());
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// 编译期嵌入的菜单栏模板图标 —— 保证无论开发 / 打包都有正确的纯黑 sparkle
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray-icon@2x.png");

/// 读取菜单栏 icon：优先用编译期嵌入的专用模板图标
fn load_tray_icon(_app: &AppHandle) -> Image<'static> {
    Image::from_bytes(TRAY_ICON_PNG)
        .expect("failed to parse embedded tray icon")
        .to_owned()
}

fn show_and_focus(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        let focused = window.is_focused().unwrap_or(false);
        if visible && focused {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}

/// 根据系统健康状态更新托盘的 title（菜单栏显示文字）+ tooltip + 菜单项。
/// 由 monitor::start_background_monitor 每 2 秒调用一次。
pub fn refresh_tray(app: &AppHandle, h: &SystemHealth) {
    // 1. 菜单栏 title：像 iStat Menus 一样显示 CPU%
    //    阈值：<60% 只显示图标不显示文字；>=60% 开始显示 CPU 数字；
    //    任一维度超过 90% 显示 "!" 警示
    let critical = h.cpu_percent >= 90.0 || h.memory_percent >= 95.0 || h.disk_percent >= 95.0;
    let warn = h.cpu_percent >= 60.0 || h.memory_percent >= 85.0 || h.disk_percent >= 90.0;

    let title_text = if critical {
        format!("! {:>3.0}%", h.cpu_percent.max(h.memory_percent))
    } else if warn {
        format!("{:>3.0}%", h.cpu_percent)
    } else {
        String::new() // 正常状态只显示图标
    };

    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_title(Some(title_text.clone()));
        let tip = format!(
            "MacSlim\nCPU {:.1}%   内存 {:.1}%   磁盘 {:.1}%",
            h.cpu_percent, h.memory_percent, h.disk_percent
        );
        let _ = tray.set_tooltip(Some(&tip));
    }

    // 2. 菜单项文字（带状态 emoji）
    if let Some(items) = app.try_state::<TrayItems>() {
        let fmt_line =
            |label: &str, pct: f32| -> String { format!("  ●  {}  {:>4.1}%", label, pct) };

        if let Ok(i) = items.cpu.lock() {
            let _ = i.set_text(fmt_line("CPU  ", h.cpu_percent));
        }
        if let Ok(i) = items.mem.lock() {
            let _ = i.set_text(fmt_line("内存", h.memory_percent));
        }
        if let Ok(i) = items.disk.lock() {
            let _ = i.set_text(fmt_line("磁盘", h.disk_percent));
        }
        if let Ok(i) = items.health_header.lock() {
            let head = if critical {
                "系统状态 · 需要关注"
            } else if warn {
                "系统状态 · 运行中"
            } else {
                "系统状态 · 正常"
            };
            let _ = i.set_text(head);
        }
    }
}
