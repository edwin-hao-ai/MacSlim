#!/usr/bin/env python3
"""临时改 tauri.conf.json 的窗口尺寸（截屏脚本用，截完由 trap 还原）。

拆成独立文件而不是塞在 shell 的 heredoc 里：heredoc 里嵌 python 容易在
多层引号里出错，而这段只需要改两个数字。
"""
import pathlib
import sys

path = pathlib.Path(__file__).resolve().parents[1] / "src-tauri/tauri.conf.json"
text = path.read_text(encoding="utf-8")

if sys.argv[1] == "fullscreen":
    text = text.replace('"fullscreen": false', '"fullscreen": true')
    print("窗口改为全屏（截图 2880x1800，App Store 认可）")
else:
    width, height = sys.argv[1], sys.argv[2]
    text = text.replace('"width": 900', f'"width": {width}')
    text = text.replace('"height": 600', f'"height": {height}')
    print(f"窗口尺寸设为 {width}x{height}")

path.write_text(text, encoding="utf-8")
