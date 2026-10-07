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

/// 托盘文案。
///
/// ## 为什么托盘要单独做一套
///
/// 托盘菜单在 Rust 里构建，拿不到前端的 i18n 上下文；而它此前是**整块硬编码
/// 中文**：英文用户在菜单栏里看到「系统状态 / 立即扫描 / 退出 MacSlim」，
/// 这是审核一眼能看见的（macOS 菜单栏属于系统 chrome，不是应用内容区）。
///
/// 语言取**系统语言**而不是应用内设置：macOS 的菜单栏/菜单项按惯例跟随系统
/// 语言，用户在应用内切到英文时菜单栏保持系统语言是符合预期的行为，也避免了
/// 为此加一条「前端把 locale 推给后端」的命令与权限。
struct TrayText {
    health_header: &'static str,
    cpu: &'static str,
    mem: &'static str,
    disk: &'static str,
    open: &'static str,
    scan: &'static str,
    optimize: &'static str,
    quit: &'static str,
    status_normal: &'static str,
    status_busy: &'static str,
    status_attention: &'static str,
    about_prefix: &'static str,
}

const ZH: TrayText = TrayText {
    health_header: "系统状态",
    cpu: "CPU",
    mem: "内存",
    disk: "磁盘",
    open: "打开 MacSlim",
    scan: "立即扫描",
    optimize: "一键优化（安全项）",
    quit: "退出 MacSlim",
    status_normal: "正常",
    status_busy: "运行中",
    status_attention: "需要关注",
    about_prefix: "关于 MacSlim",
};

const EN: TrayText = TrayText {
    health_header: "System Status",
    cpu: "CPU",
    mem: "Memory",
    disk: "Disk",
    open: "Open MacSlim",
    scan: "Scan Now",
    optimize: "One-Click Optimize (safe items)",
    quit: "Quit MacSlim",
    status_normal: "Normal",
    status_busy: "Busy",
    status_attention: "Needs attention",
    about_prefix: "About MacSlim",
};

fn text() -> &'static TrayText {
    match tauri_plugin_os::locale() {
        Some(tag) if tag.to_lowercase().starts_with("zh") => &ZH,
        _ => &EN,
    }
}

pub fn init_tray(app: &AppHandle) -> tauri::Result<()> {
    let l = text();
    // 动态状态区（只读项，靠 monitor 线程刷新）
    let health_header =
        MenuItem::with_id(app, "health_header", l.health_header, false, None::<&str>)?;
    let cpu_item = MenuItem::with_id(app, "cpu_item", format!("  {}:    —", l.cpu), false, None::<&str>)?;
    let mem_item = MenuItem::with_id(app, "mem_item", format!("  {}:   —", l.mem), false, None::<&str>)?;
    let disk_item = MenuItem::with_id(app, "disk_item", format!("  {}:   —", l.disk), false, None::<&str>)?;

    // 操作区
    let open_item = MenuItem::with_id(app, "open", l.open, true, None::<&str>)?;
    let scan_item = MenuItem::with_id(app, "scan", l.scan, true, None::<&str>)?;
    let optimize_item = MenuItem::with_id(app, "optimize", l.optimize, true, None::<&str>)?;

    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    // 版本号取编译期常量。这里曾经写死「v0.1.0」，而实际版本早就是 1.0.0 ——
    // 侧栏显示 1.0.0、菜单栏显示 0.1.0，两处自相矛盾，审核点开菜单就能看到。
    let about_item = MenuItem::with_id(
        app,
        "about",
        format!("{} v{}", l.about_prefix, env!("CARGO_PKG_VERSION")),
        false,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;

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
        let l = text();
        let tip = format!(
            "MacSlim\n{} {:.1}%   {} {:.1}%   {} {:.1}%",
            l.cpu, h.cpu_percent, l.mem, h.memory_percent, l.disk, h.disk_percent
        );
        let _ = tray.set_tooltip(Some(&tip));
    }

    // 2. 菜单项文字（带状态 emoji）
    if let Some(items) = app.try_state::<TrayItems>() {
        let l = text();
        let fmt_line =
            |label: &str, pct: f32| -> String { format!("  ●  {}  {:>4.1}%", label, pct) };

        if let Ok(i) = items.cpu.lock() {
            let _ = i.set_text(fmt_line(l.cpu, h.cpu_percent));
        }
        if let Ok(i) = items.mem.lock() {
            let _ = i.set_text(fmt_line(l.mem, h.memory_percent));
        }
        if let Ok(i) = items.disk.lock() {
            let _ = i.set_text(fmt_line(l.disk, h.disk_percent));
        }
        if let Ok(i) = items.health_header.lock() {
            let state = if critical {
                l.status_attention
            } else if warn {
                l.status_busy
            } else {
                l.status_normal
            };
            let _ = i.set_text(format!("{} · {}", l.health_header, state));
        }
    }
}
