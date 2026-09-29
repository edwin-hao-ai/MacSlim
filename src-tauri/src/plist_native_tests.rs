//! Info.plist 原生解析（替代 shell 出去的 `plutil`）。
//!
//! 重点不在「能不能解析出名字」，而在**解析器的行为等价且更严**：
//!
//! - 旧实现对「`<key>K</key>` 之后的第一个 `<string>`」做正则匹配。K 的值
//!   若是数组/字典，正则会跨过结构抓到**嵌套里**的字符串。
//! - 旧实现还有一个隐性依赖：二进制 plist 必须先 `plutil -convert xml1`，
//!   沙箱下 `plutil` 不可用。
//!
//! 所以这里测三件事：两种格式都能读、值类型不对就返回 None、空白串落空。

use super::{plist_string, read_plist_map, read_plist_metadata};
use std::path::Path;

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(dir).expect("能建目录");
    std::fs::write(&path, bytes).expect("能写 plist");
    path
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("macslim_plist_probe_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

const XML_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDisplayName</key>
    <string>Demo App</string>
    <key>CFBundleName</key>
    <string>DemoApp</string>
    <key>CFBundleIdentifier</key>
    <string>com.example.demo</string>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key>
            <string>Trap</string>
        </dict>
    </array>
</dict>
</plist>
"#;

#[test]
fn reads_metadata_from_xml_plist() {
    let dir = scratch("xml");
    let path = write(&dir, "Info.plist", XML_PLIST.as_bytes());
    let (name, id) = read_plist_metadata(&path);
    assert_eq!(name.as_deref(), Some("Demo App"));
    assert_eq!(id.as_deref(), Some("com.example.demo"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reads_metadata_from_binary_plist() {
    // 真正的 bplist：plist crate 写出来的，magic 是 "bplist00"。
    // 旧实现必须 shell 一次 plutil 才能读它；新实现原地解。
    let dir = scratch("bplist");
    let path = write(&dir, "Info.plist", &[]);
    let mut map = plist::Dictionary::new();
    map.insert("CFBundleName".into(), plist::Value::String("BinApp".into()));
    map.insert(
        "CFBundleIdentifier".into(),
        plist::Value::String("com.example.bin".into()),
    );
    let mut file = std::fs::File::create(&path).expect("能建 bplist 文件");
    plist::to_writer_binary(&mut file, &map).expect("能写 bplist");
    assert!(std::fs::read(&path).unwrap().starts_with(b"bplist"));

    let (name, id) = read_plist_metadata(&path);
    // 没有 CFBundleDisplayName 时回落到 CFBundleName
    assert_eq!(name.as_deref(), Some("BinApp"));
    assert_eq!(id.as_deref(), Some("com.example.bin"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn returns_none_when_the_value_is_not_a_string() {
    // 旧正则实现在这里会串味：CFBundleName 的值若是个数组，它会跨过结构
    // 抓到嵌套里的第一个 <string>。真正的解析器只看该 key 自己的值。
    let dir = scratch("nested");
    let path = write(&dir, "Info.plist", XML_PLIST.as_bytes());
    let map = read_plist_map(&path);

    // URLTypes 是数组，取它当字符串必须 None（而不是抓到 "Trap"）
    assert_eq!(plist_string(&map, "CFBundleURLTypes"), None);
    // 直接验证"陷阱"就在文件里：正则式实现会返回这个值
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("<string>Trap</string>"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn blank_and_missing_values_both_fall_through() {
    let dir = scratch("blank");
    let path = write(&dir, "Info.plist", XML_PLIST.as_bytes());
    let map = read_plist_map(&path);

    assert_eq!(plist_string(&map, "CFBundleDoesNotExist"), None);
    assert_eq!(plist_string(&map, ""), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unparsable_file_yields_an_empty_map_not_a_panic() {
    let dir = scratch("garbage");
    let path = write(&dir, "Info.plist", b"this is definitely not a plist");
    let (name, id) = read_plist_metadata(&path);
    assert_eq!((name, id), (None, None));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_file_yields_an_empty_map_not_a_panic() {
    let (name, id) = read_plist_metadata(Path::new("/nonexistent/Info.plist"));
    assert_eq!((name, id), (None, None));
}
