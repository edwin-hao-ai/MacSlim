//! 端口占用检测 —— 找出正在监听常见开发端口的进程。
//!
//! 用 lsof 原生命令读取。`netstat -anv` 在 macOS 上不返回 PID，
//! `lsof -iTCP -sTCP:LISTEN -P -n` 能精准拿到监听端口 + PID。

use std::collections::HashMap;
use std::process::Command;

/// 端口号 → PID 的映射。
pub fn listening_ports() -> HashMap<u16, u32> {
    let mut map = HashMap::new();

    let Ok(out) = Command::new("lsof")
        .args(["-iTCP", "-sTCP:LISTEN", "-P", "-n"])
        .output()
    else {
        return map;
    };

    let stdout = match String::from_utf8(out.stdout) {
        Ok(s) => s,
        Err(_) => return map,
    };

    // lsof 输出示例：
    // COMMAND    PID   USER   FD   TYPE   DEVICE ...  NAME
    // node    12345   user   20u  IPv4    ...       TCP *:3000 (LISTEN)
    for line in stdout.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 9 {
            continue;
        }
        let pid: u32 = match cols[1].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        // NAME 列（最后一段）形如 "*:3000" 或 "127.0.0.1:5173"
        let name = cols[8];
        if let Some(port_str) = name.rsplit(':').next() {
            if let Ok(port) = port_str.parse::<u16>() {
                // 同一 PID 可能开多个端口 —— 我们按端口索引，优先保留更「有名」的端口
                map.insert(port, pid);
            }
        }
    }

    map
}

/// 为给定的 PID 列表返回每个 PID 监听的端口列表。
pub fn ports_by_pid(pids: &[u32]) -> HashMap<u32, Vec<u16>> {
    let all = listening_ports();
    let mut out: HashMap<u32, Vec<u16>> = HashMap::new();
    for (port, pid) in all {
        if pids.contains(&pid) {
            out.entry(pid).or_default().push(port);
        }
    }
    // 排序让 UI 稳定
    for v in out.values_mut() {
        v.sort();
    }
    out
}

/// 常见开发端口 —— 如果进程监听了这些端口之一，提高「开发进程」识别优先级。
pub const COMMON_DEV_PORTS: &[u16] = &[
    80, 443, // web
    3000, 3001, 3030, 4000, 4200, 4321, // node/react/vue/nuxt/astro
    5000, 5001, 5173, 5174, // flask/vite
    6006, 6379, // storybook / redis
    7000, 7001, 7007, // misc dev
    8000, 8001, 8008, 8080, 8081, 8088, 8888, // python/django/jupyter
    9000, 9001, 9090, 9200, 9229,  // go/prometheus/elastic/node-inspect
    27017, // mongo
    5432, 5433, // postgres
    3306, 3307, // mysql
    6380, // redis alt
];
